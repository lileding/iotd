use anyhow::Result;
use iothub::config::{Config, ServerConfig, AuthConfig, StorageConfig, LoggingConfig};
use tokio::signal;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting IoTHub Server");

    let config = Config {
        server: ServerConfig {
            address: "127.0.0.1:1883".to_string(),
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
    let server = iothub::server::start(config).await?;
    
    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    info!("Received SIGINT, initiating graceful shutdown...");
    
    server.stop().await?;
    info!("Graceful shutdown completed");

    Ok(())
}
