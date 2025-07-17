use anyhow::Result;
use iothub::config::{Config, ServerConfig, AuthConfig, StorageConfig, LoggingConfig};
use iothub::server::Server;
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting IoTHub Server");

    let config = Config {
        server: ServerConfig {
            listen_addresses: vec!["tcp://127.0.0.1:1883".to_string()],
            max_connections: 0,
            session_timeout_secs: 0,
            keep_alive_timeout_secs: 0,
            max_packet_size: 0,
            retained_message_limit: 0,
        },
        auth: AuthConfig::default(),
        storage: StorageConfig::default(),
        logging: LoggingConfig::default(),
    };
    let server = Server::new(&config);
    let server_clone = Arc::clone(&server);
    
    // Spawn server task
    let server_handle = tokio::spawn(async move {
        server_clone.run().await;
    });
    
    // Handle UNIX signals
    tokio::select! {
        // SIGINT (Ctrl+C) - graceful shutdown
        _ = signal::ctrl_c() => {
            info!("Received SIGINT, initiating graceful shutdown...");
            server.shutdown().await;
            info!("Graceful shutdown completed");
        }
        
        // SIGTERM - immediate quit
        _ = async {
            #[cfg(unix)]
            {
                let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
                sigterm.recv().await
            }
            #[cfg(not(unix))]
            {
                // On non-Unix systems, just wait forever (SIGTERM not available)
                std::future::pending::<()>().await
            }
        } => {
            warn!("Received SIGTERM, quitting immediately");
            std::process::exit(0);
        }
        
        // Server completed normally
        _ = server_handle => {
            info!("Server completed normally");
        }
    }

    Ok(())
}
