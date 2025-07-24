use anyhow::Result;
use iotd::config::{Config, ServerConfig};
use tokio::signal;
use tracing::{info, Level};
use tracing_subscriber;

// Version information
const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_REVISION: &str = env!("GIT_REVISION");

fn print_version() {
    println!("iotd {}-{}", VERSION, GIT_REVISION);
}

fn print_help() {
    println!("IoTD - High-Performance MQTT Daemon");
    println!();
    println!("A high-performance MQTT v3.1.1 server daemon implementation in Rust.");
    println!("Supports thousands of concurrent connections with low latency message routing.");
    println!();
    println!("By default, the server listens on 0.0.0.0:1883 for MQTT connections.");
    println!();
    println!("USAGE:");
    println!("    iotd [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print help information");
    println!("    -v, --version    Print version information");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse arguments manually to handle version and help
    let args: Vec<String> = std::env::args().collect();
    
    for arg in &args[1..] {
        match arg.as_str() {
            "-v" | "--version" => {
                print_version();
                return Ok(());
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            _ => {}
        }
    }
    
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting IoTD Server v{}-{}", VERSION, GIT_REVISION);

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