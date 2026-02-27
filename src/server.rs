use crate::auth;
use crate::broker::Broker;
use crate::config::Config;
use crate::storage;
use crate::transport::{self, AsyncListener, Protocol, TcpAsyncListener};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("Server failed to start: {0}")]
    StartupFailed(String),
    #[error("Server is already running")]
    AlreadyRunning,
    #[error("Server is not running")]
    NotRunning,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Storage error: {0}")]
    Storage(#[from] storage::StorageError),
    #[error("Auth error: {0}")]
    Auth(#[from] auth::AuthError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ServerState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

struct LifecycleState {
    state: ServerState,
    shutdown_token: Option<CancellationToken>,
    listener_handles: Vec<JoinHandle<()>>,
    addresses: Vec<String>,
}

/// Static function to start a server with the given configuration
pub async fn start(config: Config) -> Result<Server> {
    let server = Server::new(config)?;
    server.start().await?;
    Ok(server)
}

pub struct Server {
    config: Config,
    broker: Arc<Broker>,
    lifecycle: Mutex<LifecycleState>,
}

impl Server {
    pub fn new(config: Config) -> Result<Self> {
        let storage = storage::new(&config.persistence)?;
        info!("Storage backend: {:?}", config.persistence.backend);

        let authenticator = auth::new(&config.auth)?;
        info!("Auth backend: {:?}", config.auth.backend);

        let authorizer = auth::new_authorizer(&config.acl)?;
        info!("ACL backend: {:?}", config.acl.backend);

        Ok(Self {
            broker: Broker::new(config.clone(), storage, authenticator, authorizer),
            config,
            lifecycle: Mutex::new(LifecycleState {
                state: ServerState::Stopped,
                shutdown_token: None,
                listener_handles: Vec::new(),
                addresses: Vec::new(),
            }),
        })
    }

    pub async fn start(&self) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;

        if lifecycle.state != ServerState::Stopped {
            return Err(ServerError::AlreadyRunning);
        }
        lifecycle.state = ServerState::Starting;

        info!("Starting server");

        // Create fresh token for this run
        let shutdown_token = CancellationToken::new();
        let mut handles = Vec::new();
        let mut addresses = Vec::new();

        // Build TLS acceptor once if any TLS listener is configured
        let tls_acceptor = if self.config.listen.iter().any(|l| l.starts_with("tls://")) {
            let tls_config = self.config.tls.as_ref().ok_or_else(|| {
                lifecycle.state = ServerState::Stopped;
                ServerError::StartupFailed(
                    "TLS listener configured but [tls] section missing from config".into(),
                )
            })?;
            Some(transport::build_tls_acceptor(tls_config).map_err(|e| {
                lifecycle.state = ServerState::Stopped;
                ServerError::StartupFailed(format!("Failed to initialize TLS: {e}"))
            })?)
        } else {
            None
        };

        for listen_addr in &self.config.listen {
            let (protocol, addr) = Protocol::from_url(listen_addr).map_err(|e| {
                lifecycle.state = ServerState::Stopped;
                ServerError::StartupFailed(format!("Invalid listen address '{}': {e}", listen_addr))
            })?;

            let listener: Box<dyn AsyncListener> = match protocol {
                Protocol::Tcp => Box::new(TcpAsyncListener::bind(&addr).await.map_err(|e| {
                    lifecycle.state = ServerState::Stopped;
                    ServerError::StartupFailed(format!("Failed to bind TCP to {addr}: {e}"))
                })?),
                Protocol::TcpTls => {
                    let acceptor = tls_acceptor.clone().unwrap();
                    Box::new(
                        transport::TlsAsyncListener::bind(&addr, acceptor)
                            .await
                            .map_err(|e| {
                                lifecycle.state = ServerState::Stopped;
                                ServerError::StartupFailed(format!(
                                    "Failed to bind TLS to {addr}: {e}"
                                ))
                            })?,
                    )
                }
                _ => {
                    lifecycle.state = ServerState::Stopped;
                    return Err(ServerError::StartupFailed(format!(
                        "Unsupported protocol in '{}'",
                        listen_addr
                    )));
                }
            };

            let bound_addr = listener.local_addr().await.map_err(|e| {
                lifecycle.state = ServerState::Stopped;
                ServerError::StartupFailed(format!("Failed to get local address: {e}"))
            })?;

            let protocol_name = match protocol {
                Protocol::Tcp => "TCP",
                Protocol::TcpTls => "TLS",
                _ => "Unknown",
            };
            info!("Listening on {} ({})", bound_addr, protocol_name);
            addresses.push(bound_addr.to_string());

            let token_clone = shutdown_token.clone();
            let broker = Arc::clone(&self.broker);
            handles.push(tokio::spawn(async move {
                Self::run_listener(broker, listener, token_clone).await;
            }));
        }

        // Store everything
        lifecycle.shutdown_token = Some(shutdown_token);
        lifecycle.listener_handles = handles;
        lifecycle.addresses = addresses;
        lifecycle.state = ServerState::Running;

        info!("Server started successfully");
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;

        if lifecycle.state != ServerState::Running {
            return Err(ServerError::NotRunning);
        }
        lifecycle.state = ServerState::Stopping;

        info!("Stopping server");

        // Cancel shutdown token
        if let Some(token) = lifecycle.shutdown_token.take() {
            token.cancel();
        }

        // Wait for all listener tasks to complete
        for handle in lifecycle.listener_handles.drain(..) {
            handle.await.unwrap_or_else(|e| {
                error!("Error waiting for listener task: {e}");
            });
        }

        // Clear all sessions (broker persists, retained messages preserved)
        self.broker.clean_all_sessions().await;

        // Clear and reset - ready for restart
        lifecycle.addresses.clear();
        lifecycle.state = ServerState::Stopped;

        info!("Server stopped");
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        self.lifecycle.lock().await.state == ServerState::Running
    }

    /// Returns the first listener's bound address (for backward compatibility)
    pub async fn address(&self) -> Option<String> {
        self.lifecycle.lock().await.addresses.first().cloned()
    }

    /// Returns all listener bound addresses
    pub async fn addresses(&self) -> Vec<String> {
        self.lifecycle.lock().await.addresses.clone()
    }

    async fn run_listener(
        broker: Arc<Broker>,
        listener: Box<dyn AsyncListener>,
        shutdown_token: CancellationToken,
    ) {
        // Main listener loop
        loop {
            tokio::select! {
                // Accept new client connections
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok(stream) => {
                            info!("New client connected");

                            // Add client to broker
                            broker.add_client(stream).await;
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {e}");
                        }
                    }
                }

                // Handle shutdown signal
                _ = shutdown_token.cancelled() => {
                    info!("Listener received shutdown signal");
                    break;
                }
            }
        }

        // Close the listener to prevent new connections
        if let Err(e) = listener.close().await {
            error!("Failed to close listener: {e}");
        }

        info!("Listener loop completed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(port: u16) -> Config {
        Config {
            listen: vec![format!("127.0.0.1:{}", port)],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_server_lifecycle() {
        let server = Server::new(test_config(0)).unwrap();

        // Initially stopped
        assert!(!server.is_running().await);

        // Start
        server.start().await.unwrap();
        assert!(server.is_running().await);
        assert!(server.address().await.is_some());

        // Double start should fail
        assert!(matches!(
            server.start().await,
            Err(ServerError::AlreadyRunning)
        ));

        // Stop
        server.stop().await.unwrap();
        assert!(!server.is_running().await);

        // Double stop should fail
        assert!(matches!(server.stop().await, Err(ServerError::NotRunning)));
    }

    #[tokio::test]
    async fn test_server_restart() {
        let server = Server::new(test_config(0)).unwrap();

        // First run
        server.start().await.unwrap();
        let addr1 = server.address().await;
        assert!(server.is_running().await);

        server.stop().await.unwrap();
        assert!(!server.is_running().await);
        assert!(server.address().await.is_none());

        // Restart with fresh state
        server.start().await.unwrap();
        let addr2 = server.address().await;
        assert!(server.is_running().await);

        // Both addresses should be valid (port 0 means different ports)
        assert!(addr1.is_some());
        assert!(addr2.is_some());

        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_stop_before_start() {
        let server = Server::new(test_config(0)).unwrap();

        // Stop without start should fail
        assert!(matches!(server.stop().await, Err(ServerError::NotRunning)));
    }
}
