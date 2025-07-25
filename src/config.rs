use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub address: String,
    pub retained_message_limit: usize,
    pub max_retransmission_limit: u32,
    pub retransmission_interval_ms: u64,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:1883".to_string(),
            retained_message_limit: 10000,
            max_retransmission_limit: 10,
            retransmission_interval_ms: 5000, // 5 seconds
        }
    }
}

impl ServerConfig {
    /// Get the retransmission interval with protection (minimum 500ms, 0 means disabled)
    pub fn get_retransmission_interval_ms(&self) -> u64 {
        if self.retransmission_interval_ms == 0 {
            0 // Disabled
        } else {
            self.retransmission_interval_ms.max(500) // Minimum 500ms
        }
    }
}


impl Config {
    pub fn retransmission_interval(&self) -> Duration {
        Duration::from_millis(self.server.retransmission_interval_ms)
    }
}

impl ServerConfig {
    pub fn retransmission_interval(&self) -> Duration {
        Duration::from_millis(self.retransmission_interval_ms)
    }
}
