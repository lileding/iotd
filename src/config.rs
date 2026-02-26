use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen address (default: "127.0.0.1:1883")
    #[serde(default = "default_listen")]
    pub listen: String,

    /// Maximum number of retained messages (default: 10000)
    #[serde(default = "default_retained_message_limit")]
    pub retained_message_limit: usize,

    /// Maximum retransmission attempts for QoS=1 (default: 10)
    #[serde(default = "default_max_retransmission_limit")]
    pub max_retransmission_limit: u32,

    /// Retransmission interval in milliseconds (default: 5000)
    #[serde(default = "default_retransmission_interval_ms")]
    pub retransmission_interval_ms: u64,

    /// Persistence configuration
    #[serde(default)]
    pub persistence: PersistenceConfig,

    /// Authentication configuration
    #[serde(default)]
    pub auth: AuthConfig,
}

fn default_listen() -> String {
    "127.0.0.1:1883".to_string()
}

fn default_retained_message_limit() -> usize {
    10000
}

fn default_max_retransmission_limit() -> u32 {
    10
}

fn default_retransmission_interval_ms() -> u64 {
    5000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            retained_message_limit: default_retained_message_limit(),
            max_retransmission_limit: default_max_retransmission_limit(),
            retransmission_interval_ms: default_retransmission_interval_ms(),
            persistence: PersistenceConfig::default(),
            auth: AuthConfig::default(),
        }
    }
}

impl Config {
    /// Get the retransmission interval as Duration
    pub fn retransmission_interval(&self) -> Duration {
        Duration::from_millis(self.retransmission_interval_ms)
    }

    /// Get the retransmission interval with protection (minimum 500ms, 0 means disabled)
    pub fn get_retransmission_interval_ms(&self) -> u64 {
        if self.retransmission_interval_ms == 0 {
            0 // Disabled
        } else {
            self.retransmission_interval_ms.max(500) // Minimum 500ms
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Memory,
    Sqlite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceConfig {
    /// Storage backend: "memory" or "sqlite" (default: "memory")
    #[serde(default)]
    pub backend: StorageBackend,

    /// Path to SQLite database file (only used when backend = "sqlite")
    #[serde(default = "default_database_path")]
    pub database_path: PathBuf,

    /// Session expiry in seconds (0 = never expire, default: 86400 = 24 hours)
    #[serde(default = "default_session_expiry_seconds")]
    pub session_expiry_seconds: u64,
}

fn default_database_path() -> PathBuf {
    PathBuf::from("iotd.db")
}

fn default_session_expiry_seconds() -> u64 {
    86400 // 24 hours
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::default(),
            database_path: default_database_path(),
            session_expiry_seconds: default_session_expiry_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthBackend {
    #[default]
    #[serde(rename = "allowall")]
    AllowAll,
    #[serde(rename = "passwordfile")]
    PasswordFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Auth backend: "allowall" or "passwordfile" (default: "allowall")
    #[serde(default)]
    pub backend: AuthBackend,

    /// Path to password file (only used when backend = "passwordfile")
    #[serde(default = "default_password_file_path")]
    pub password_file: PathBuf,
}

fn default_password_file_path() -> PathBuf {
    PathBuf::from("passwd")
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            backend: AuthBackend::default(),
            password_file: default_password_file_path(),
        }
    }
}
