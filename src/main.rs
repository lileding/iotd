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
    println!("USAGE:");
    println!("    iotd [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help              Print help information");
    println!("    -v, --version           Print version information");
    println!("    -l, --listen <ADDRESS>  Listen address (default: 127.0.0.1:1883)");
    println!();
    println!("EXAMPLES:");
    println!("    iotd                    # Listen on 127.0.0.1:1883");
    println!("    iotd -l 0.0.0.0:1883    # Listen on all interfaces");
    println!("    iotd -l [::]:1883       # Listen on all interfaces (IPv6)");
    println!("    iotd -l [::1]:1883      # Listen on localhost (IPv6)");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse arguments manually to handle version, help, and listen address
    let args: Vec<String> = std::env::args().collect();
    let mut listen_address = "127.0.0.1:1883".to_string();
    
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--version" => {
                print_version();
                return Ok(());
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-l" | "--listen" => {
                if i + 1 < args.len() {
                    listen_address = args[i + 1].clone();
                    i += 1; // Skip the next argument as it's the address
                } else {
                    eprintln!("Error: -l/--listen requires an address argument");
                    print_help();
                    return Ok(());
                }
            }
            arg => {
                eprintln!("Error: Unknown argument '{}'", arg);
                print_help();
                return Ok(());
            }
        }
        i += 1;
    }
    
    // Use RUST_LOG env var if set, otherwise default to INFO
    let log_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<Level>().ok())
        .unwrap_or(Level::INFO);
    
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .init();

    info!("Starting IoTD Server v{}-{}", VERSION, GIT_REVISION);
    info!("Listening on: {}", listen_address);

    let config = Config {
        server: ServerConfig {
            address: listen_address,
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