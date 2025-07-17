use crate::broker::Broker;
use crate::config::Config;
use crate::session::Session;
use crate::router::Router;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub struct Server {
    config: Config,
    brokers: DashMap<String, Arc<Broker>>,
    sessions: DashMap<String, Arc<Session>>,
    router: Router,
    draining: RwLock<bool>,
    shutdown_token: CancellationToken,
    completed_token: CancellationToken,
}

impl Server {
    pub fn new(config: &Config) -> Arc<Self> {
        Arc::new(Self {
            config: config.clone(),
            brokers: DashMap::new(),
            sessions: DashMap::new(),
            router: Router::new(),
            draining: RwLock::new(false),
            shutdown_token: CancellationToken::new(),
            completed_token: CancellationToken::new(),
        })
    }

    pub async fn run(self: Arc<Self>) {
        info!("Starting Server");

        // Start brokers for each configured address
        for bind_address in &self.config.server.listen_addresses {
            let broker = Broker::new(bind_address, Arc::clone(&self));
            self.brokers.insert(bind_address.clone(), Arc::clone(&broker));

            let bind_address = bind_address.clone();
            tokio::spawn(async move {
                broker.run().await;
                info!("Broker {} completed", &bind_address);
            });
        }

        info!("Server started successfully");

        self.shutdown_token.cancelled().await;
        info!("Server received shutdown signal");

        // Enable drain mode to prevent new session registrations
        *self.draining.write().await = true;

        // Wait for all brokers to shutdown
        for broker in self.brokers.iter() {
            broker.shutdown().await;
        }
        self.brokers.clear();

        // Clear all sessions
        for session in self.sessions.iter() {
            session.value().shutdown().await;
            self.router.unsubscribe_all(&session.value()).await;
        }
        self.sessions.clear();

        // Mark as completed
        self.completed_token.cancel();

        info!("Server run() completed");
    }

    pub async fn shutdown(&self) {
        info!("Shutting down server");

        // Send shutdown signal
        self.shutdown_token.cancel();

        // Wait for completion
        self.completed_token.cancelled().await;
        info!("Server shutdown completed");
    }

    pub async fn register_session(&self, session: Arc<Session>) -> Result<()> {
        // Check if we're in drain mode
        let draining = self.draining.read().await;
        if *draining {
            return Err(anyhow::anyhow!("Server is in drain mode, rejecting new session"));
        }

        let session_id = session.session_id().await;

        // Handle sessionId conflicts using atomic swap
        if let Some(existing_session) = self.sessions.insert(session_id.clone(), session) {
            info!("Removing existing session {}", &session_id);
            existing_session.shutdown().await;
            self.router.unsubscribe_all(&existing_session).await;
        }

        info!("Registered session {}", &session_id);
        Ok(())
    }

    pub async fn unregister_session(&self, session_id: &str) {
        // When a session ends (either on its own or during server shutdown),
        // it calls this function to remove itself from the server's records.
        if let Some((_, session)) = self.sessions.remove(session_id) {
            self.router.unsubscribe_all(&session).await;
            info!("Unregistered session {}", session_id);
        }
    }
}

