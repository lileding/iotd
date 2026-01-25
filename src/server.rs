use crate::broker::Broker;
use crate::config::Config;
use crate::transport::{AsyncListener, TcpAsyncListener};
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
    server_handle: Option<JoinHandle<()>>,
    address: Option<String>,
}

/// Static function to start a server with the given configuration
pub async fn start(config: Config) -> Result<Server> {
    let server = Server::new(config);
    server.start().await?;
    Ok(server)
}

pub struct Server {
    config: Config,
    broker: Arc<Broker>,
    lifecycle: Mutex<LifecycleState>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            broker: Broker::new(config.clone()),
            config,
            lifecycle: Mutex::new(LifecycleState {
                state: ServerState::Stopped,
                shutdown_token: None,
                server_handle: None,
                address: None,
            }),
        }
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

        // Bind the listener directly to the address
        let listener = TcpAsyncListener::bind(&self.config.listen)
            .await
            .map_err(|e| {
                lifecycle.state = ServerState::Stopped;
                ServerError::StartupFailed(format!("Failed to bind to {}: {e}", self.config.listen))
            })?;

        // Get the actual bound address (useful for port 0)
        let bound_addr = listener.local_addr().await.map_err(|e| {
            lifecycle.state = ServerState::Stopped;
            ServerError::StartupFailed(format!("Failed to get local address: {e}"))
        })?;

        info!("Server listening on {}", bound_addr);

        // Spawn the server task with token clone
        let token_clone = shutdown_token.clone();
        let broker = Arc::clone(&self.broker);
        let handle = tokio::spawn(async move {
            Self::run_server(broker, listener, token_clone).await;
        });

        // Store everything
        lifecycle.shutdown_token = Some(shutdown_token);
        lifecycle.server_handle = Some(handle);
        lifecycle.address = Some(bound_addr.to_string());
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

        // Wait for the server task to complete
        if let Some(handle) = lifecycle.server_handle.take() {
            handle.await.unwrap_or_else(|e| {
                error!("Error waiting for server task: {e}");
            });
        }

        // Clear all sessions (broker persists, retained messages preserved)
        self.broker.clean_all_sessions().await;

        // Clear and reset - ready for restart
        lifecycle.address = None;
        lifecycle.state = ServerState::Stopped;

        info!("Server stopped");
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        self.lifecycle.lock().await.state == ServerState::Running
    }

    pub async fn address(&self) -> Option<String> {
        self.lifecycle.lock().await.address.clone()
    }

    async fn run_server(
        broker: Arc<Broker>,
        listener: TcpAsyncListener,
        shutdown_token: CancellationToken,
    ) {
        // Main server loop
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
                    info!("Server received shutdown signal");
                    break;
                }
            }
        }

        // Close the listener to prevent new connections
        if let Err(e) = listener.close().await {
            error!("Failed to close listener: {e}");
        }

        info!("Server loop completed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(port: u16) -> Config {
        Config {
            listen: format!("127.0.0.1:{}", port),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_server_lifecycle() {
        let server = Server::new(test_config(0));

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
        let server = Server::new(test_config(0));

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
        let server = Server::new(test_config(0));

        // Stop without start should fail
        assert!(matches!(server.stop().await, Err(ServerError::NotRunning)));
    }
}
