use crate::transport::AsyncStream;
use crate::server::Server;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};
use tracing::{error, info};
use uuid::Uuid;

pub struct Session {
    session_id: String,
    client_id: String,
    clean_session: bool,
    keep_alive: u16,
    stream: RwLock<Box<dyn AsyncStream>>,
    server: Arc<Server>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}

impl Session {
    pub fn new(
        stream: Box<dyn AsyncStream>,
        server: Arc<Server>,
    ) -> Arc<Self> {
        let session_id = Uuid::new_v4().to_string();
        let client_id = format!("__iothub_{}", session_id);
        
        Arc::new(Self {
            session_id,
            client_id, // Default client ID, will be updated after CONNECT packet if provided
            clean_session: true,
            keep_alive: 0,
            stream: RwLock::new(stream),
            server: Arc::clone(&server),
            shutdown: Arc::new(Notify::new()),
            completed: Arc::new(Notify::new()),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Session {} starting", self.session_id);

        // Session handling loop
        loop {
            tokio::select! {
                // Handle incoming packets from client
                result = self.handle_client_packet() => {
                    match result {
                        Ok(should_continue) => {
                            if !should_continue {
                                info!("Session {} ending normally", self.session_id);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Session {} error: {}", self.session_id, e);
                            break;
                        }
                    }
                }
                
                // Handle shutdown signal
                _ = self.shutdown.notified() => {
                    info!("Session {} shutting down", self.session_id);
                    break;
                }
            }
        }

        // Cleanup: unregister session from server
        self.server.unregister_session(&self.session_id).await;
        
        // Close the stream
        if let Err(e) = self.stream.write().await.close().await {
            error!("Failed to close stream for session {}: {}", self.session_id, e);
        }

        info!("Session {} completed", self.session_id);
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.shutdown.notify_waiters();
        self.completed.notified().await;
    }

    async fn handle_client_packet(&self) -> Result<bool> {
        // TODO: Implement MQTT packet handling
        // For now, just return false to end the session
        // This would normally:
        // 1. Read MQTT packet from stream
        // 2. Parse and validate packet
        // 3. Handle different packet types (CONNECT, PUBLISH, SUBSCRIBE, etc.)
        // 4. Update session state
        // 5. Send responses back to client
        // 6. Route messages through server
        
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        Ok(false) // End session for now
    }

    pub fn set_client_id(&mut self, client_id: String) {
        if client_id.is_empty() {
            // Keep the default client ID if provided client_id is empty
            return;
        }
        self.client_id = client_id;
    }

    pub fn set_clean_session(&mut self, clean_session: bool) {
        self.clean_session = clean_session;
    }

    pub fn set_keep_alive(&mut self, keep_alive: u16) {
        self.keep_alive = keep_alive;
    }
}
