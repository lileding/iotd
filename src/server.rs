use crate::auth;
use crate::broker::Broker;
use crate::config::Config;
use crate::session::SessionFuture;
use crate::storage;
use crate::transport::{self, AsyncListener, AsyncStream, Protocol, TcpAsyncListener};
use futures::future::join_all;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("Server failed to start: {0}")]
    StartupFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Storage error: {0}")]
    Storage(#[from] storage::StorageError),
    #[error("Auth error: {0}")]
    Auth(#[from] auth::AuthError),
}

pub struct Server {
    config: Config,
    broker: Broker,
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
        })
    }

    pub async fn run(&self, cancel: &CancellationToken) -> Result<()> {
        let (listeners, _) = self.bind().await?;
        self.serve(listeners, cancel).await
    }

    async fn bind(&self) -> Result<(Vec<Box<dyn AsyncListener>>, Vec<String>)> {
        info!("Binding listeners");

        let tls_acceptor = if self.config.listen.iter().any(|l| l.starts_with("tls://")) {
            let tls_config = self.config.tls.as_ref().ok_or_else(|| {
                ServerError::StartupFailed(
                    "TLS listener configured but [tls] section missing from config".into(),
                )
            })?;
            Some(transport::build_tls_acceptor(tls_config).map_err(|e| {
                ServerError::StartupFailed(format!("Failed to initialize TLS: {e}"))
            })?)
        } else {
            None
        };

        let mut listeners = Vec::new();
        let mut addresses = Vec::new();

        for listen_addr in &self.config.listen {
            let (protocol, addr) = Protocol::from_url(listen_addr).map_err(|e| {
                ServerError::StartupFailed(format!("Invalid listen address '{}': {e}", listen_addr))
            })?;

            let listener: Box<dyn AsyncListener> = match protocol {
                Protocol::Tcp => Box::new(TcpAsyncListener::bind(&addr).await.map_err(|e| {
                    ServerError::StartupFailed(format!("Failed to bind TCP to {addr}: {e}"))
                })?),
                Protocol::TcpTls => {
                    let acceptor = tls_acceptor.clone().unwrap();
                    Box::new(
                        transport::TlsAsyncListener::bind(&addr, acceptor)
                            .await
                            .map_err(|e| {
                                ServerError::StartupFailed(format!(
                                    "Failed to bind TLS to {addr}: {e}"
                                ))
                            })?,
                    )
                }
                _ => {
                    return Err(ServerError::StartupFailed(format!(
                        "Unsupported protocol in '{}'",
                        listen_addr
                    )));
                }
            };

            let bound_addr = listener.local_addr().await.map_err(|e| {
                ServerError::StartupFailed(format!("Failed to get local address: {e}"))
            })?;

            let protocol_name = match protocol {
                Protocol::Tcp => "TCP",
                Protocol::TcpTls => "TLS",
                _ => "Unknown",
            };
            info!("Listening on {} ({})", bound_addr, protocol_name);
            addresses.push(bound_addr.to_string());
            listeners.push(listener);
        }

        Ok((listeners, addresses))
    }

    async fn serve(
        &self,
        listeners: Vec<Box<dyn AsyncListener>>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        info!("Server started successfully");

        let (stream_tx, mut stream_rx) = mpsc::channel::<Box<dyn AsyncStream>>(64);
        let mut sessions: FuturesUnordered<SessionFuture> = FuturesUnordered::new();

        // Run all listeners concurrently, feeding accepted streams into the channel
        let listeners_done = {
            let futs: Vec<_> = listeners
                .into_iter()
                .map(|listener| {
                    let tx = stream_tx.clone();
                    let cancel = cancel.clone();
                    async move {
                        Self::accept_loop(listener, tx, cancel).await;
                    }
                })
                .collect();
            drop(stream_tx); // drop our copy so channel closes when all listeners exit
            join_all(futs)
        };
        tokio::pin!(listeners_done);

        // Main orchestration loop
        loop {
            tokio::select! {
                Some(stream) = stream_rx.recv() => {
                    info!("New client connected");
                    let session_fut = self.broker.add_client(stream).await;
                    sessions.push(session_fut);
                }
                Some(()) = sessions.next(), if !sessions.is_empty() => {
                    // session completed, automatically removed from set
                }
                _ = &mut listeners_done => {
                    // all listeners stopped (cancel was triggered)
                    break;
                }
            }
        }

        // Cancel remaining sessions and wait for them to finish
        self.broker.cancel_all_sessions().await;
        while sessions.next().await.is_some() {}

        info!("Server stopped");
        Ok(())
    }

    async fn accept_loop(
        listener: Box<dyn AsyncListener>,
        stream_tx: mpsc::Sender<Box<dyn AsyncStream>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok(stream) => {
                            if stream_tx.send(stream).await.is_err() {
                                break; // receiver dropped
                            }
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {e}");
                        }
                    }
                }
                _ = cancel.cancelled() => {
                    info!("Listener received shutdown signal");
                    break;
                }
            }
        }

        if let Err(e) = listener.close().await {
            error!("Failed to close listener: {e}");
        }

        info!("Listener loop completed");
    }
}

// --- Test/convenience helper: spawns server in background, returns a handle ---

pub struct ServerHandle {
    cancel: CancellationToken,
    addresses: Vec<String>,
}

impl ServerHandle {
    pub async fn address(&self) -> Option<String> {
        self.addresses.first().cloned()
    }

    pub async fn addresses(&self) -> Vec<String> {
        self.addresses.clone()
    }

    pub async fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        Ok(())
    }
}

pub async fn start(config: Config) -> Result<ServerHandle> {
    let server = Server::new(config)?;
    let (listeners, addresses) = server.bind().await?;

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        if let Err(e) = server.serve(listeners, &cancel_clone).await {
            error!("Server error: {e}");
        }
    });

    Ok(ServerHandle { cancel, addresses })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            listen: vec!["127.0.0.1:0".to_string()],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_server_start_stop() {
        let handle = start(test_config()).await.unwrap();
        assert!(handle.address().await.is_some());
        handle.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_server_binds_address() {
        let handle = start(test_config()).await.unwrap();
        let addr = handle.address().await.unwrap();
        assert!(addr.contains("127.0.0.1:"));
        handle.stop().await.unwrap();
    }
}
