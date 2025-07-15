pub mod broker;
pub mod client;
pub mod session;

use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

use self::broker::Broker;
use self::client::ClientHandler;

pub struct IoTHub {
    address: String,
    broker: Arc<Broker>,
}

impl IoTHub {
    pub async fn new(address: String) -> Result<Self> {
        let broker = Arc::new(Broker::new());
        
        Ok(IoTHub {
            address,
            broker,
        })
    }

    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(&self.address).await?;
        info!("IoTHub MQTT server listening on {}", self.address);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New client connected from {}", addr);
                    let broker = Arc::clone(&self.broker);
                    
                    tokio::spawn(async move {
                        let mut client_handler = ClientHandler::new(stream, broker);
                        if let Err(e) = client_handler.handle().await {
                            error!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}