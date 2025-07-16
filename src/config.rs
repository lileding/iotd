use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen_addresses: Vec<String>,
    pub max_connections: usize,
    pub session_timeout_secs: u64,
    pub keep_alive_timeout_secs: u64,
    pub max_packet_size: usize,
    pub retained_message_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub backend: String,
    pub config: toml::Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub backend: String,
    pub config: toml::Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            storage: StorageConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec!["tcp://0.0.0.0:1883".to_string()],
            max_connections: 10000,
            session_timeout_secs: 300,
            keep_alive_timeout_secs: 60,
            max_packet_size: 1024 * 1024, // 1MB
            retained_message_limit: 10000,
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: "none".to_string(),
            config: toml::Table::new(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: "memory".to_string(),
            config: toml::Table::new(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "text".to_string(),
        }
    }
}

impl Config {
    pub fn session_timeout(&self) -> Duration {
        Duration::from_secs(self.server.session_timeout_secs)
    }

    pub fn keep_alive_timeout(&self) -> Duration {
        Duration::from_secs(self.server.keep_alive_timeout_secs)
    }
}

impl ServerConfig {
    pub fn session_timeout(&self) -> Duration {
        Duration::from_secs(self.session_timeout_secs)
    }

    pub fn keep_alive_timeout(&self) -> Duration {
        Duration::from_secs(self.keep_alive_timeout_secs)
    }
}
