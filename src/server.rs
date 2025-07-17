use crate::config::Config;
use crate::session::Session;
use crate::router::Router;
use crate::transport::{AsyncListener, TcpAsyncListener};
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock, Mutex};
use tracing::{error, info};

pub struct Server {
    config: Config,
    brokers: DashMap<String, Arc<Broker>>,
    sessions: Mutex<DashMap<String, Arc<Session>>>,
    router: Router,
    draining: RwLock<bool>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}

impl Server {
    pub fn new(config: &Config) -> Arc<Self> {
        Arc::new(Self {
            config: config.clone(),
            brokers: DashMap::new(),
            sessions: Mutex::new(DashMap::new()),
            router: Router::new(),
            draining: RwLock::new(false),
            shutdown: Arc::new(Notify::new()),
            completed: Arc::new(Notify::new()),
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Starting Server");
        
        // Start brokers for each configured address
        for bind_address in &self.config.server.listen_addresses {
            let broker = Broker::new(bind_address, Arc::clone(&self));
            self.brokers.insert(bind_address.clone(), Arc::clone(&broker));
            
            let bind_address = bind_address.clone();
            tokio::spawn(async move {
                if let Err(e) = broker.run().await {
                    error!("Broker {} failed: {}", &bind_address, e);
                }
            });
        }

        info!("Server started successfully");
       
        self.shutdown.notified().await;
        info!("Server received shutdown signal");
        
        // Enable drain mode to prevent new session registrations
        *self.draining.write().await = true;
        
        // Wait for all brokers to shutdown
        for broker in self.brokers.iter() {
            broker.shutdown().await;
        }
        self.brokers.clear();
        
        // Clear all sessions
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.iter() {
                session.value().shutdown().await;
                self.router.unsubscribe_all(&session.value()).await;
            }
            sessions.clear();
        }
        
        // Mark as completed and notify
        self.completed.notify_waiters();
        
        info!("Server run() completed");
        Ok(())
    }

    pub async fn shutdown(&self) {
        info!("Shutting down server");
               
        // Send shutdown signal
        self.shutdown.notify_waiters();
        
        // Wait for completion
        self.completed.notified().await;
        info!("Server shutdown completed");
    }

    pub async fn register_session(&self, session: Arc<Session>) -> Result<()> {
        // Check if we're in drain mode
        let draining = self.draining.read().await;
        if *draining {
            return Err(anyhow::anyhow!("Server is in drain mode, rejecting new session"));
        }

        let client_id = session.client_id().await;
        let session_id = session.session_id().to_string();
        
        // Handle client ID conflicts using atomic swap
        let sessions = self.sessions.lock().await;
        if let Some(existing_session) = sessions.insert(client_id.clone(), session) {
            info!("Removing existing session {} for client {}", &existing_session.session_id(), &client_id);
            existing_session.shutdown().await;
            self.router.unsubscribe_all(&existing_session).await;
        }
        
        info!("Registered session {} for client {}", &session_id, &client_id);
        Ok(())
    }

    pub async fn unregister_session(&self, session_id: &str) {
        let draining = self.draining.read().await;
        if *draining {
            return;
        }

        // Find session by session_id and remove it
        let sessions = self.sessions.lock().await;
        if let Some((_, session)) = sessions.remove(session_id) {
            session.shutdown().await;
            self.router.unsubscribe_all(&session).await;
        }
    }
}

struct Broker {
    bind_address: String,
    server: Arc<Server>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}

impl Broker {
    pub fn new<S: AsRef<str>>(
        bind_address: S,
        server: Arc<Server>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bind_address: bind_address.as_ref().to_string(),
            server,
            shutdown: Arc::new(Notify::new()),
            completed: Arc::new(Notify::new()),
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        // Parse protocol from bind_address
        let (protocol, addr) = parse_address(&self.bind_address)?;
        
        // Create listener
        let listener = self.create_listener(&protocol, &addr).await?;
        let local_addr = listener.local_addr().await?;
        
        info!("Broker listening on {} (bound to {})", self.bind_address, local_addr);
        
        // Main accept loop
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok(stream) => {
                            let peer_addr = stream.peer_addr().unwrap_or_else(|_| "unknown".parse().unwrap());
                            info!("New connection from {}", peer_addr);
                            
                            let session = Session::new(stream, Arc::clone(&self.server));
                            let session_run = Arc::clone(&session);
                            tokio::spawn(async move {
                                if let Err(e) = session_run.run().await {
                                    error!("Session error: {}", e);
                                }
                            });
                            self.server.register_session(session).await?;
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                            break; // Exit the loop on accept failure
                        }
                    }
                }
                
                _ = self.shutdown.notified() => {
                    info!("Broker shutting down gracefully");
                    break;
                }
            }
        }

        // Close listener
        listener.close().await?;
        
        // Mark as completed and notify (always succeeds, never blocks)
        self.completed.notify_waiters();
        
        info!("Broker run() completed");
        Ok(())
    }

    pub async fn shutdown(&self) {
        // Send shutdown signal (non-blocking)
        self.shutdown.notify_waiters();
        
        // Wait for completion
        self.completed.notified().await;
        
        info!("Broker shutdown complete");
    }

    async fn create_listener(&self, protocol: &str, addr: &str) -> Result<Box<dyn AsyncListener>> {
        match protocol {
            "tcp" => {
                let listener = TcpAsyncListener::bind(addr).await?;
                Ok(Box::new(listener))
            }
            _ => {
                anyhow::bail!("Protocol '{}' not implemented yet", protocol);
            }
        }
    }
}

// Helper function to parse address
fn parse_address(bind_address: &str) -> Result<(String, String)> {
    if let Some(addr) = bind_address.strip_prefix("tcp://") {
        Ok(("tcp".to_string(), addr.to_string()))
    } else if let Some(addr) = bind_address.strip_prefix("ws://") {
        Ok(("ws".to_string(), addr.to_string()))
    } else if let Some(addr) = bind_address.strip_prefix("wss://") {
        Ok(("wss".to_string(), addr.to_string()))
    } else {
        anyhow::bail!("Unsupported protocol in address: {}", bind_address)
    }
}
