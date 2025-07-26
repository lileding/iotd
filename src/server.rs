use crate::broker::Broker;
use crate::config::Config;
use crate::transport::{AsyncListener, TcpAsyncListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
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

/// Static function to start a server with the given configuration
pub async fn start(config: Config) -> Result<Server> {
    let server = Server::new(config);
    server.start().await?;
    Ok(server)
}

pub struct Server {
    config: Config,
    broker: Arc<Broker>,
    shutdown_token: CancellationToken,
    running: AtomicBool,
    server_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    address: RwLock<Option<String>>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            broker: Broker::new(config.clone()),
            config,
            shutdown_token: CancellationToken::new(),
            running: AtomicBool::new(false),
            server_handle: Mutex::new(None),
            address: RwLock::new(None),
        }
    }

    pub async fn start(&self) -> Result<()> {
        if self.running.load(Ordering::Acquire) {
            return Err(ServerError::AlreadyRunning);
        }

        info!("Starting Server");

        // Bind the listener directly to the address
        let listener = TcpAsyncListener::bind(&self.config.server.address)
            .await
            .map_err(|e| {
                ServerError::StartupFailed(format!(
                    "Failed to bind to {}: {e}",
                    self.config.server.address
                ))
            })?;

        // Get the actual bound address (useful for port 0)
        let bound_addr = listener
            .local_addr()
            .await
            .map_err(|e| ServerError::StartupFailed(format!("Failed to get local address: {e}")))?;

        *self.address.write().await = Some(bound_addr.to_string());

        info!("Server listening on {}", bound_addr);

        // Spawn the server task
        let shutdown_token = self.shutdown_token.clone();
        let broker = Arc::clone(&self.broker);
        let handle = tokio::spawn(async move {
            Self::run_server(broker, listener, shutdown_token).await;
        });

        // Store the handle
        *self.server_handle.lock().await = Some(handle);

        self.running.store(true, Ordering::Release);
        info!("Server started successfully");
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(ServerError::NotRunning);
        }

        info!("Stopping server");

        // Send shutdown signal
        self.shutdown_token.cancel();

        // Wait for the server task to complete
        if let Some(handle) = self.server_handle.lock().await.take() {
            handle.await.unwrap_or_else(|e| {
                error!("Error in waiting for server task {e}");
            });
        }

        // Clear all sessions
        self.broker.clean_all_sessions().await;

        // Mark as not running
        self.running.store(false, Ordering::Release);

        info!("Server stopped");
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub async fn address(&self) -> Option<String> {
        self.address.read().await.clone()
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
