use crate::storage::types::{
    PersistedInflightMessage, PersistedRetainedMessage, PersistedSession, PersistedSubscription,
    PersistedWillMessage, StoredQoS,
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result, Transaction};
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Invalid datetime in {table}.{field} for {record_id}: '{value}'")]
    InvalidDateTime {
        table: &'static str,
        field: &'static str,
        record_id: String,
        value: String,
    },
}

/// SQLite-based persistence storage for MQTT sessions
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

fn parse_datetime(
    s: &str,
    table: &'static str,
    field: &'static str,
    record_id: &str,
) -> std::result::Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StorageError::InvalidDateTime {
            table,
            field,
            record_id: record_id.to_string(),
            value: s.to_string(),
        })
}

impl SqliteStorage {
    /// Create a new SQLite storage with the given database path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.initialize_schema()?;
        Ok(storage)
    }

    /// Create an in-memory SQLite storage (useful for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.initialize_schema()?;
        Ok(storage)
    }

    /// Initialize database schema
    fn initialize_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r#"
            -- Sessions table
            CREATE TABLE IF NOT EXISTS sessions (
                client_id TEXT PRIMARY KEY,
                next_packet_id INTEGER NOT NULL,
                keep_alive INTEGER NOT NULL,
                will_topic TEXT,
                will_payload BLOB,
                will_qos INTEGER,
                will_retain INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Subscriptions table
            CREATE TABLE IF NOT EXISTS subscriptions (
                client_id TEXT NOT NULL,
                topic_filter TEXT NOT NULL,
                qos INTEGER NOT NULL,
                PRIMARY KEY (client_id, topic_filter)
            );

            -- In-flight messages (QoS=1 awaiting PUBACK)
            CREATE TABLE IF NOT EXISTS inflight_messages (
                client_id TEXT NOT NULL,
                packet_id INTEGER NOT NULL,
                topic TEXT NOT NULL,
                payload BLOB NOT NULL,
                qos INTEGER NOT NULL,
                retain INTEGER NOT NULL,
                retry_count INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (client_id, packet_id)
            );

            -- Retained messages (server-wide)
            CREATE TABLE IF NOT EXISTS retained_messages (
                topic TEXT PRIMARY KEY,
                payload BLOB NOT NULL,
                qos INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Indexes for efficient lookups
            CREATE INDEX IF NOT EXISTS idx_subscriptions_client
                ON subscriptions(client_id);
            CREATE INDEX IF NOT EXISTS idx_inflight_client
                ON inflight_messages(client_id);
            "#,
        )?;

        Ok(())
    }

    /// Helper to delete all data for a client within a transaction
    fn delete_client_data(tx: &Transaction, client_id: &str) -> Result<()> {
        tx.execute(
            "DELETE FROM inflight_messages WHERE client_id = ?1",
            params![client_id],
        )?;
        tx.execute(
            "DELETE FROM subscriptions WHERE client_id = ?1",
            params![client_id],
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE client_id = ?1",
            params![client_id],
        )?;
        Ok(())
    }

    // ========== Session Operations ==========

    /// Save or update a session
    pub fn save_session(&self, session: &PersistedSession) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO sessions (
                client_id, next_packet_id, keep_alive,
                will_topic, will_payload, will_qos, will_retain,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(client_id) DO UPDATE SET
                next_packet_id = excluded.next_packet_id,
                keep_alive = excluded.keep_alive,
                will_topic = excluded.will_topic,
                will_payload = excluded.will_payload,
                will_qos = excluded.will_qos,
                will_retain = excluded.will_retain,
                updated_at = excluded.updated_at
            "#,
            params![
                session.client_id,
                session.next_packet_id,
                session.keep_alive,
                session.will_message.as_ref().map(|w| w.topic.clone()),
                session.will_message.as_ref().map(|w| w.payload.to_vec()),
                session.will_message.as_ref().map(|w| w.qos.as_u8()),
                session.will_message.as_ref().map(|w| w.retain as i32),
                session.created_at.to_rfc3339(),
                session.updated_at.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    /// Load a session by client ID
    pub fn load_session(
        &self,
        client_id: &str,
    ) -> std::result::Result<Option<PersistedSession>, StorageError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT client_id, next_packet_id, keep_alive,
                   will_topic, will_payload, will_qos, will_retain,
                   created_at, updated_at
            FROM sessions WHERE client_id = ?1
            "#,
        )?;

        let mut rows = stmt.query(params![client_id])?;

        if let Some(row) = rows.next()? {
            let client_id: String = row.get(0)?;
            let will_topic: Option<String> = row.get(3)?;
            let will_payload: Option<Vec<u8>> = row.get(4)?;
            let will_qos: Option<u8> = row.get(5)?;
            let will_retain: Option<i32> = row.get(6)?;
            let created_at_str: String = row.get(7)?;
            let updated_at_str: String = row.get(8)?;

            let will_message = match (will_topic, will_payload, will_qos, will_retain) {
                (Some(topic), Some(payload), Some(qos), Some(retain)) => {
                    Some(PersistedWillMessage {
                        topic,
                        payload: Bytes::from(payload),
                        qos: StoredQoS::from_u8(qos).unwrap_or(StoredQoS::AtMostOnce),
                        retain: retain != 0,
                    })
                }
                _ => None,
            };

            Ok(Some(PersistedSession {
                client_id: client_id.clone(),
                next_packet_id: row.get::<_, i32>(1)? as u16,
                keep_alive: row.get::<_, i32>(2)? as u16,
                will_message,
                created_at: parse_datetime(&created_at_str, "sessions", "created_at", &client_id)?,
                updated_at: parse_datetime(&updated_at_str, "sessions", "updated_at", &client_id)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Delete a session and all associated data (uses transaction)
    pub fn delete_session(&self, client_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        Self::delete_client_data(&tx, client_id)?;

        tx.commit()
    }

    /// Check if a session exists
    pub fn session_exists(&self, client_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE client_id = ?1",
            params![client_id],
            |row| row.get(0),
        )?;

        Ok(count > 0)
    }

    /// Delete expired sessions older than the given datetime (uses transaction)
    pub fn delete_expired_sessions(&self, older_than: DateTime<Utc>) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let older_than_str = older_than.to_rfc3339();

        let tx = conn.transaction()?;

        // Get client IDs of expired sessions
        let expired_clients: Vec<String> = {
            let mut stmt = tx.prepare("SELECT client_id FROM sessions WHERE updated_at < ?1")?;
            let results: Vec<String> = stmt
                .query_map(params![older_than_str], |row| row.get(0))?
                .collect::<Result<Vec<_>>>()?;
            results
        };

        let count = expired_clients.len();

        // Delete associated data for each expired session
        for client_id in &expired_clients {
            Self::delete_client_data(&tx, client_id)?;
        }

        tx.commit()?;

        Ok(count)
    }

    // ========== Subscription Operations ==========

    /// Save a subscription
    pub fn save_subscription(&self, sub: &PersistedSubscription) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO subscriptions (client_id, topic_filter, qos)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(client_id, topic_filter) DO UPDATE SET
                qos = excluded.qos
            "#,
            params![sub.client_id, sub.topic_filter, sub.qos.as_u8()],
        )?;

        Ok(())
    }

    /// Save multiple subscriptions for a client (uses transaction)
    pub fn save_subscriptions(&self, subscriptions: &[PersistedSubscription]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO subscriptions (client_id, topic_filter, qos)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(client_id, topic_filter) DO UPDATE SET
                    qos = excluded.qos
                "#,
            )?;

            for sub in subscriptions {
                stmt.execute(params![sub.client_id, sub.topic_filter, sub.qos.as_u8()])?;
            }
        }

        tx.commit()
    }

    /// Load all subscriptions for a client
    pub fn load_subscriptions(&self, client_id: &str) -> Result<Vec<PersistedSubscription>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT client_id, topic_filter, qos FROM subscriptions WHERE client_id = ?1",
        )?;

        let subs = stmt
            .query_map(params![client_id], |row| {
                let qos_val: u8 = row.get(2)?;
                Ok(PersistedSubscription {
                    client_id: row.get(0)?,
                    topic_filter: row.get(1)?,
                    qos: StoredQoS::from_u8(qos_val).unwrap_or(StoredQoS::AtMostOnce),
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(subs)
    }

    /// Delete a subscription
    pub fn delete_subscription(&self, client_id: &str, topic_filter: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM subscriptions WHERE client_id = ?1 AND topic_filter = ?2",
            params![client_id, topic_filter],
        )?;

        Ok(())
    }

    /// Delete all subscriptions for a client
    pub fn delete_all_subscriptions(&self, client_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM subscriptions WHERE client_id = ?1",
            params![client_id],
        )?;

        Ok(())
    }

    // ========== In-flight Message Operations ==========

    /// Save an in-flight message
    pub fn save_inflight_message(&self, msg: &PersistedInflightMessage) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO inflight_messages (
                client_id, packet_id, topic, payload, qos, retain, retry_count, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(client_id, packet_id) DO UPDATE SET
                retry_count = excluded.retry_count
            "#,
            params![
                msg.client_id,
                msg.packet_id,
                msg.topic,
                msg.payload.to_vec(),
                msg.qos.as_u8(),
                msg.retain as i32,
                msg.retry_count,
                msg.created_at.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    /// Load all in-flight messages for a client
    pub fn load_inflight_messages(
        &self,
        client_id: &str,
    ) -> std::result::Result<Vec<PersistedInflightMessage>, StorageError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT client_id, packet_id, topic, payload, qos, retain, retry_count, created_at
            FROM inflight_messages WHERE client_id = ?1
            ORDER BY created_at ASC
            "#,
        )?;

        let mut messages = Vec::new();
        let mut rows = stmt.query(params![client_id])?;

        while let Some(row) = rows.next()? {
            let client_id: String = row.get(0)?;
            let packet_id: i32 = row.get(1)?;
            let payload: Vec<u8> = row.get(3)?;
            let qos_val: u8 = row.get(4)?;
            let retain_val: i32 = row.get(5)?;
            let created_at_str: String = row.get(7)?;

            // Use client_id:packet_id as record identifier for error context
            let record_id = format!("{}:{}", client_id, packet_id);

            messages.push(PersistedInflightMessage {
                client_id,
                packet_id: packet_id as u16,
                topic: row.get(2)?,
                payload: Bytes::from(payload),
                qos: StoredQoS::from_u8(qos_val).unwrap_or(StoredQoS::AtLeastOnce),
                retain: retain_val != 0,
                retry_count: row.get::<_, i32>(6)? as u32,
                created_at: parse_datetime(
                    &created_at_str,
                    "inflight_messages",
                    "created_at",
                    &record_id,
                )?,
            });
        }

        Ok(messages)
    }

    /// Delete an in-flight message (when PUBACK received)
    pub fn delete_inflight_message(&self, client_id: &str, packet_id: u16) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM inflight_messages WHERE client_id = ?1 AND packet_id = ?2",
            params![client_id, packet_id],
        )?;

        Ok(())
    }

    /// Delete all in-flight messages for a client
    pub fn delete_all_inflight_messages(&self, client_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM inflight_messages WHERE client_id = ?1",
            params![client_id],
        )?;

        Ok(())
    }

    // ========== Retained Message Operations ==========

    /// Save a retained message
    pub fn save_retained_message(&self, msg: &PersistedRetainedMessage) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO retained_messages (topic, payload, qos, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(topic) DO UPDATE SET
                payload = excluded.payload,
                qos = excluded.qos,
                updated_at = excluded.updated_at
            "#,
            params![
                msg.topic,
                msg.payload.to_vec(),
                msg.qos.as_u8(),
                msg.updated_at.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    /// Load a retained message by topic
    pub fn load_retained_message(
        &self,
        topic: &str,
    ) -> std::result::Result<Option<PersistedRetainedMessage>, StorageError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT topic, payload, qos, updated_at FROM retained_messages WHERE topic = ?1",
        )?;

        let mut rows = stmt.query(params![topic])?;

        if let Some(row) = rows.next()? {
            let topic: String = row.get(0)?;
            let payload: Vec<u8> = row.get(1)?;
            let qos_val: u8 = row.get(2)?;
            let updated_at_str: String = row.get(3)?;

            Ok(Some(PersistedRetainedMessage {
                topic: topic.clone(),
                payload: Bytes::from(payload),
                qos: StoredQoS::from_u8(qos_val).unwrap_or(StoredQoS::AtMostOnce),
                updated_at: parse_datetime(
                    &updated_at_str,
                    "retained_messages",
                    "updated_at",
                    &topic,
                )?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Load all retained messages
    pub fn load_all_retained_messages(
        &self,
    ) -> std::result::Result<Vec<PersistedRetainedMessage>, StorageError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt =
            conn.prepare("SELECT topic, payload, qos, updated_at FROM retained_messages")?;

        let mut messages = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let topic: String = row.get(0)?;
            let payload: Vec<u8> = row.get(1)?;
            let qos_val: u8 = row.get(2)?;
            let updated_at_str: String = row.get(3)?;

            messages.push(PersistedRetainedMessage {
                topic: topic.clone(),
                payload: Bytes::from(payload),
                qos: StoredQoS::from_u8(qos_val).unwrap_or(StoredQoS::AtMostOnce),
                updated_at: parse_datetime(
                    &updated_at_str,
                    "retained_messages",
                    "updated_at",
                    &topic,
                )?,
            });
        }

        Ok(messages)
    }

    /// Delete a retained message
    pub fn delete_retained_message(&self, topic: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM retained_messages WHERE topic = ?1",
            params![topic],
        )?;

        Ok(())
    }

    /// Count retained messages
    pub fn count_retained_messages(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM retained_messages", [], |row| {
            row.get(0)
        })?;

        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_crud() {
        let storage = SqliteStorage::in_memory().unwrap();
        let now = Utc::now();

        // Create session
        let session = PersistedSession {
            client_id: "test-client".to_string(),
            next_packet_id: 100,
            keep_alive: 60,
            will_message: Some(PersistedWillMessage {
                topic: "will/topic".to_string(),
                payload: Bytes::from("goodbye"),
                qos: StoredQoS::AtLeastOnce,
                retain: true,
            }),
            created_at: now,
            updated_at: now,
        };

        storage.save_session(&session).unwrap();

        // Load session
        let loaded = storage.load_session("test-client").unwrap().unwrap();
        assert_eq!(loaded.client_id, "test-client");
        assert_eq!(loaded.next_packet_id, 100);
        assert_eq!(loaded.keep_alive, 60);
        assert!(loaded.will_message.is_some());

        let will = loaded.will_message.unwrap();
        assert_eq!(will.topic, "will/topic");
        assert_eq!(will.payload, Bytes::from("goodbye"));
        assert_eq!(will.qos, StoredQoS::AtLeastOnce);
        assert!(will.retain);

        // Check exists
        assert!(storage.session_exists("test-client").unwrap());
        assert!(!storage.session_exists("nonexistent").unwrap());

        // Delete session
        storage.delete_session("test-client").unwrap();
        assert!(!storage.session_exists("test-client").unwrap());
    }

    #[test]
    fn test_subscription_crud() {
        let storage = SqliteStorage::in_memory().unwrap();

        // Save subscriptions
        let subs = vec![
            PersistedSubscription {
                client_id: "client1".to_string(),
                topic_filter: "topic/+/data".to_string(),
                qos: StoredQoS::AtLeastOnce,
            },
            PersistedSubscription {
                client_id: "client1".to_string(),
                topic_filter: "sensor/#".to_string(),
                qos: StoredQoS::AtMostOnce,
            },
        ];

        storage.save_subscriptions(&subs).unwrap();

        // Load subscriptions
        let loaded = storage.load_subscriptions("client1").unwrap();
        assert_eq!(loaded.len(), 2);

        // Delete one subscription
        storage
            .delete_subscription("client1", "topic/+/data")
            .unwrap();
        let loaded = storage.load_subscriptions("client1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].topic_filter, "sensor/#");

        // Delete all subscriptions
        storage.delete_all_subscriptions("client1").unwrap();
        let loaded = storage.load_subscriptions("client1").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_inflight_message_crud() {
        let storage = SqliteStorage::in_memory().unwrap();
        let now = Utc::now();

        // Save in-flight message
        let msg = PersistedInflightMessage {
            client_id: "client1".to_string(),
            packet_id: 1,
            topic: "test/topic".to_string(),
            payload: Bytes::from("hello"),
            qos: StoredQoS::AtLeastOnce,
            retain: false,
            retry_count: 0,
            created_at: now,
        };

        storage.save_inflight_message(&msg).unwrap();

        // Load in-flight messages
        let loaded = storage.load_inflight_messages("client1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].packet_id, 1);
        assert_eq!(loaded[0].topic, "test/topic");
        assert_eq!(loaded[0].payload, Bytes::from("hello"));

        // Delete in-flight message
        storage.delete_inflight_message("client1", 1).unwrap();
        let loaded = storage.load_inflight_messages("client1").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_retained_message_crud() {
        let storage = SqliteStorage::in_memory().unwrap();
        let now = Utc::now();

        // Save retained message
        let msg = PersistedRetainedMessage {
            topic: "sensor/temperature".to_string(),
            payload: Bytes::from("25.5"),
            qos: StoredQoS::AtMostOnce,
            updated_at: now,
        };

        storage.save_retained_message(&msg).unwrap();

        // Load retained message
        let loaded = storage
            .load_retained_message("sensor/temperature")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.topic, "sensor/temperature");
        assert_eq!(loaded.payload, Bytes::from("25.5"));

        // Load all retained messages
        let all = storage.load_all_retained_messages().unwrap();
        assert_eq!(all.len(), 1);

        // Count retained messages
        assert_eq!(storage.count_retained_messages().unwrap(), 1);

        // Delete retained message
        storage
            .delete_retained_message("sensor/temperature")
            .unwrap();
        assert!(storage
            .load_retained_message("sensor/temperature")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_session_with_subscriptions_cascade_delete() {
        let storage = SqliteStorage::in_memory().unwrap();
        let now = Utc::now();

        // Create session with subscriptions and in-flight messages
        let session = PersistedSession {
            client_id: "client1".to_string(),
            next_packet_id: 1,
            keep_alive: 60,
            will_message: None,
            created_at: now,
            updated_at: now,
        };
        storage.save_session(&session).unwrap();

        let sub = PersistedSubscription {
            client_id: "client1".to_string(),
            topic_filter: "test/#".to_string(),
            qos: StoredQoS::AtLeastOnce,
        };
        storage.save_subscription(&sub).unwrap();

        let msg = PersistedInflightMessage {
            client_id: "client1".to_string(),
            packet_id: 1,
            topic: "test/topic".to_string(),
            payload: Bytes::from("data"),
            qos: StoredQoS::AtLeastOnce,
            retain: false,
            retry_count: 0,
            created_at: now,
        };
        storage.save_inflight_message(&msg).unwrap();

        // Delete session should cascade to subscriptions and in-flight messages
        storage.delete_session("client1").unwrap();

        assert!(!storage.session_exists("client1").unwrap());
        assert!(storage.load_subscriptions("client1").unwrap().is_empty());
        assert!(storage
            .load_inflight_messages("client1")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_expired_sessions_cleanup() {
        let storage = SqliteStorage::in_memory().unwrap();
        use chrono::Duration;

        let old_time = Utc::now() - Duration::hours(2);
        let recent_time = Utc::now();

        // Create an old session
        let old_session = PersistedSession {
            client_id: "old-client".to_string(),
            next_packet_id: 1,
            keep_alive: 60,
            will_message: None,
            created_at: old_time,
            updated_at: old_time,
        };
        storage.save_session(&old_session).unwrap();

        // Create a recent session
        let recent_session = PersistedSession {
            client_id: "recent-client".to_string(),
            next_packet_id: 1,
            keep_alive: 60,
            will_message: None,
            created_at: recent_time,
            updated_at: recent_time,
        };
        storage.save_session(&recent_session).unwrap();

        // Delete sessions older than 1 hour ago
        let cutoff = Utc::now() - Duration::hours(1);
        let deleted = storage.delete_expired_sessions(cutoff).unwrap();

        assert_eq!(deleted, 1);
        assert!(!storage.session_exists("old-client").unwrap());
        assert!(storage.session_exists("recent-client").unwrap());
    }

    #[test]
    fn test_invalid_datetime_error_message() {
        let storage = SqliteStorage::in_memory().unwrap();

        // Manually insert a session with invalid datetime
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute(
                r#"
                INSERT INTO sessions (
                    client_id, next_packet_id, keep_alive,
                    created_at, updated_at
                ) VALUES ('bad-client', 1, 60, 'not-a-date', '2024-01-01T00:00:00Z')
                "#,
                [],
            )
            .unwrap();
        }

        // Try to load the session
        let result = storage.load_session("bad-client");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("sessions"));
        assert!(err_msg.contains("created_at"));
        assert!(err_msg.contains("bad-client"));
        assert!(err_msg.contains("not-a-date"));
    }
}
