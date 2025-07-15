use anyhow::Result;
use iothub::server::IoTHub;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting IoTHub MQTT Server");

    let server = IoTHub::new("0.0.0.0:1883".to_string()).await?;
    server.run().await?;

    Ok(())
}