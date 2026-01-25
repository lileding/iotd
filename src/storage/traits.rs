use crate::storage::types::{
    PersistedInflightMessage, PersistedRetainedMessage, PersistedSession, PersistedSubscription,
};
use chrono::{DateTime, Utc};
use std::fmt::Debug;

/// Result type for storage operations
pub type StorageResult<T> = std::result::Result<T, StorageError>;

/// Error type for storage operations
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Storage error: {0}")]
    Internal(String),
    #[error("Invalid datetime in {table}.{field} for {record_id}: '{value}'")]
    InvalidDateTime {
        table: &'static str,
        field: &'static str,
        record_id: String,
        value: String,
    },
}

/// Unified storage trait for all persistence operations
pub trait Storage: Send + Sync + Debug {
    // ========== Session Operations ==========

    /// Save session state atomically (session + subscriptions + in-flight messages)
    /// For clean_session=false clients on disconnect
    fn save_session(
        &self,
        session: &PersistedSession,
        subscriptions: &[PersistedSubscription],
        inflight: &[PersistedInflightMessage],
    ) -> StorageResult<()>;

    /// Load a session by client ID
    fn load_session(&self, client_id: &str) -> StorageResult<Option<PersistedSession>>;

    /// Load all subscriptions for a client
    fn load_subscriptions(&self, client_id: &str) -> StorageResult<Vec<PersistedSubscription>>;

    /// Load all in-flight messages for a client
    fn load_inflight_messages(
        &self,
        client_id: &str,
    ) -> StorageResult<Vec<PersistedInflightMessage>>;

    /// Delete a session and all associated data (subscriptions, in-flight messages)
    fn delete_session(&self, client_id: &str) -> StorageResult<()>;

    /// Check if a session exists
    fn session_exists(&self, client_id: &str) -> StorageResult<bool>;

    /// Delete expired sessions older than the given datetime
    fn delete_expired_sessions(&self, older_than: DateTime<Utc>) -> StorageResult<usize>;

    // ========== Retained Message Operations ==========

    /// Save a retained message (empty payload deletes)
    fn save_retained_message(&self, msg: &PersistedRetainedMessage) -> StorageResult<()>;

    /// Load a retained message by exact topic
    fn load_retained_message(&self, topic: &str)
        -> StorageResult<Option<PersistedRetainedMessage>>;

    /// Load all retained messages (caller handles wildcard filtering)
    fn load_all_retained_messages(&self) -> StorageResult<Vec<PersistedRetainedMessage>>;

    /// Delete a retained message
    fn delete_retained_message(&self, topic: &str) -> StorageResult<()>;

    /// Count retained messages
    fn count_retained_messages(&self) -> StorageResult<usize>;
}
