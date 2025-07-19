use anyhow::Result;
use iotd::config::{Config, ServerConfig};
use tokio::signal;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting IoTD Server");

    let config = Config {
        server: ServerConfig {
            address: "0.0.0.0:1883".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let server = iotd::server::start(config).await?;
    
    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    info!("Received SIGINT, initiating graceful shutdown...");
    
    server.stop().await?;
    info!("Graceful shutdown completed");

    Ok(())
}
