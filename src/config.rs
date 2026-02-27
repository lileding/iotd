use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen addresses with optional protocol prefix (default: ["127.0.0.1:1883"])
    /// Supports: "tcp://addr:port", "tls://addr:port", or bare "addr:port" (treated as TCP)
    #[serde(
        default = "default_listen",
        deserialize_with = "deserialize_listen",
        serialize_with = "serialize_listen"
    )]
    pub listen: Vec<String>,

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

    /// ACL (authorization) configuration
    #[serde(default)]
    pub acl: AclConfig,

    /// TLS configuration (required when any listen address uses tls://)
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

fn default_listen() -> Vec<String> {
    vec!["127.0.0.1:1883".to_string()]
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

/// Custom deserializer that accepts both a single string and an array of strings
fn deserialize_listen<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct ListenVisitor;

    impl<'de> de::Visitor<'de> for ListenVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or array of strings")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Vec<String>, E> {
            Ok(vec![value.to_string()])
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut vec = Vec::new();
            while let Some(value) = seq.next_element()? {
                vec.push(value);
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(ListenVisitor)
}

/// Custom serializer: serialize single-element vec as string, multi as array
fn serialize_listen<S>(listen: &Vec<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    if listen.len() == 1 {
        serializer.serialize_str(&listen[0])
    } else {
        let mut seq = serializer.serialize_seq(Some(listen.len()))?;
        for addr in listen {
            seq.serialize_element(addr)?;
        }
        seq.end()
    }
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
            acl: AclConfig::default(),
            tls: None,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AclBackend {
    #[default]
    #[serde(rename = "allowall")]
    AllowAll,
    #[serde(rename = "aclfile")]
    AclFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclConfig {
    /// ACL backend: "allowall" or "aclfile" (default: "allowall")
    #[serde(default)]
    pub backend: AclBackend,

    /// Path to ACL file (only used when backend = "aclfile")
    #[serde(default = "default_acl_file_path")]
    pub acl_file: PathBuf,
}

fn default_acl_file_path() -> PathBuf {
    PathBuf::from("acl.conf")
}

impl Default for AclConfig {
    fn default() -> Self {
        Self {
            backend: AclBackend::default(),
            acl_file: default_acl_file_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to PEM-encoded certificate file (or certificate chain)
    pub cert_file: PathBuf,

    /// Path to PEM-encoded private key file
    pub key_file: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listen_string_backward_compat() {
        let config: Config = toml::from_str(r#"listen = "127.0.0.1:1883""#).unwrap();
        assert_eq!(config.listen, vec!["127.0.0.1:1883"]);
    }

    #[test]
    fn test_listen_array() {
        let config: Config =
            toml::from_str(r#"listen = ["tcp://0.0.0.0:1883", "tls://0.0.0.0:8883"]"#).unwrap();
        assert_eq!(
            config.listen,
            vec!["tcp://0.0.0.0:1883", "tls://0.0.0.0:8883"]
        );
    }

    #[test]
    fn test_tls_config_present() {
        let config: Config = toml::from_str(
            r#"
            [tls]
            cert_file = "server.crt"
            key_file = "server.key"
            "#,
        )
        .unwrap();
        let tls = config.tls.unwrap();
        assert_eq!(tls.cert_file, PathBuf::from("server.crt"));
        assert_eq!(tls.key_file, PathBuf::from("server.key"));
    }

    #[test]
    fn test_tls_config_absent() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.tls.is_none());
    }

    #[test]
    fn test_default_listen() {
        let config = Config::default();
        assert_eq!(config.listen, vec!["127.0.0.1:1883"]);
    }
}
