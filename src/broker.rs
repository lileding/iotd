use crate::server::Server;
use crate::session::Session;
use crate::transport::{AsyncListener, TcpAsyncListener};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub struct Broker {
    bind_address: String,
    server: Arc<Server>,
    shutdown_token: CancellationToken,
    completed_token: CancellationToken,
    half_connected_sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Broker {
    pub fn new<S: AsRef<str>>(
        bind_address: S,
        server: Arc<Server>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bind_address: bind_address.as_ref().to_string(),
            server,
            shutdown_token: CancellationToken::new(),
            completed_token: CancellationToken::new(),
            half_connected_sessions: Mutex::new(HashMap::new()),
        })
    }

    pub async fn run(self: Arc<Self>) {
        // Parse protocol from bind_address
        let (protocol, addr) = match parse_address(&self.bind_address) {
            Ok(parsed) => parsed,
            Err(e) => {
                error!("Failed to parse bind address {}: {}", self.bind_address, e);
                self.completed_token.cancel();
                return;
            }
        };
        
        // Create listener
        let listener = match self.create_listener(&protocol, &addr).await {
            Ok(listener) => listener,
            Err(e) => {
                error!("Failed to create listener for {}: {}", self.bind_address, e);
                self.completed_token.cancel();
                return;
            }
        };
        
        let local_addr = match listener.local_addr().await {
            Ok(addr) => addr,
            Err(e) => {
                error!("Failed to get local address: {}", e);
                self.completed_token.cancel();
                return;
            }
        };
        
        info!("Broker listening on {} (bound to {})", self.bind_address, local_addr);
        
        // Main accept loop
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok(stream) => {
                            let peer_addr = stream.peer_addr().unwrap_or_else(|_| "unknown".parse().unwrap());
                            info!("New connection from {}", peer_addr);
                            
                            let session = Session::new(stream, Arc::clone(&self.server), Arc::clone(&self));
                            let session_id = session.session_id().await;
                            
                            // Track as half-connected session
                            self.half_connected_sessions.lock().await.insert(session_id.clone(), Arc::clone(&session));
                            
                            tokio::spawn(async move {
                                session.run().await;
                            });
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                            break; // Exit the loop on accept failure
                        }
                    }
                }
                
                _ = self.shutdown_token.cancelled() => {
                    info!("Broker shutting down gracefully");
                    break;
                }
            }
        }

        // Close listener
        if let Err(e) = listener.close().await {
            error!("Failed to close listener: {}", e);
        }
        
        // Clean up half-connected sessions
        let sessions_to_shutdown = {
            let mut sessions = self.half_connected_sessions.lock().await;
            let temp_map = sessions.clone();
            sessions.clear();
            temp_map
        };
        
        info!("Cleaning up {} half-connected sessions", sessions_to_shutdown.len());
        
        // Now safely shutdown all sessions from the swapped map
        for (_, session) in sessions_to_shutdown {
            session.shutdown().await;
        }
        
        // Mark as completed
        self.completed_token.cancel();
        
        info!("Broker run() completed");
    }

    pub async fn shutdown(&self) {
        // Send shutdown signal
        self.shutdown_token.cancel();
        
        // Wait for completion
        self.completed_token.cancelled().await;
        
        info!("Broker shutdown complete");
    }

    pub async fn remove_half_connected_session(&self, session_id: &str) {
        self.half_connected_sessions.lock().await.remove(session_id);
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