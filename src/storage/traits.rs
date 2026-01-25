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

    /// Save or update a session
    fn save_session(&self, session: &PersistedSession) -> StorageResult<()>;

    /// Load a session by client ID
    fn load_session(&self, client_id: &str) -> StorageResult<Option<PersistedSession>>;

    /// Delete a session and all associated data
    fn delete_session(&self, client_id: &str) -> StorageResult<()>;

    /// Check if a session exists
    fn session_exists(&self, client_id: &str) -> StorageResult<bool>;

    /// Delete expired sessions older than the given datetime
    fn delete_expired_sessions(&self, older_than: DateTime<Utc>) -> StorageResult<usize>;

    // ========== Subscription Operations ==========

    /// Save a subscription
    fn save_subscription(&self, sub: &PersistedSubscription) -> StorageResult<()>;

    /// Save multiple subscriptions
    fn save_subscriptions(&self, subscriptions: &[PersistedSubscription]) -> StorageResult<()>;

    /// Load all subscriptions for a client
    fn load_subscriptions(&self, client_id: &str) -> StorageResult<Vec<PersistedSubscription>>;

    /// Delete a subscription
    fn delete_subscription(&self, client_id: &str, topic_filter: &str) -> StorageResult<()>;

    /// Delete all subscriptions for a client
    fn delete_all_subscriptions(&self, client_id: &str) -> StorageResult<()>;

    // ========== In-flight Message Operations ==========

    /// Save an in-flight message
    fn save_inflight_message(&self, msg: &PersistedInflightMessage) -> StorageResult<()>;

    /// Load all in-flight messages for a client
    fn load_inflight_messages(
        &self,
        client_id: &str,
    ) -> StorageResult<Vec<PersistedInflightMessage>>;

    /// Delete an in-flight message (when PUBACK received)
    fn delete_inflight_message(&self, client_id: &str, packet_id: u16) -> StorageResult<()>;

    /// Delete all in-flight messages for a client
    fn delete_all_inflight_messages(&self, client_id: &str) -> StorageResult<()>;

    // ========== Retained Message Operations ==========

    /// Save a retained message
    fn save_retained_message(&self, msg: &PersistedRetainedMessage) -> StorageResult<()>;

    /// Load a retained message by topic
    fn load_retained_message(&self, topic: &str)
        -> StorageResult<Option<PersistedRetainedMessage>>;

    /// Load all retained messages
    fn load_all_retained_messages(&self) -> StorageResult<Vec<PersistedRetainedMessage>>;

    /// Delete a retained message
    fn delete_retained_message(&self, topic: &str) -> StorageResult<()>;

    /// Count retained messages
    fn count_retained_messages(&self) -> StorageResult<usize>;
}
