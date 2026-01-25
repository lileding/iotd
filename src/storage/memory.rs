use crate::storage::traits::{Storage, StorageResult};
use crate::storage::types::{
    PersistedInflightMessage, PersistedRetainedMessage, PersistedSession, PersistedSubscription,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory storage implementation
/// All data is stored in HashMaps and lost on server restart
#[derive(Debug)]
pub struct InMemoryStorage {
    sessions: RwLock<HashMap<String, PersistedSession>>,
    subscriptions: RwLock<HashMap<String, Vec<PersistedSubscription>>>, // client_id -> subs
    inflight: RwLock<HashMap<String, Vec<PersistedInflightMessage>>>,   // client_id -> messages
    retained: RwLock<HashMap<String, PersistedRetainedMessage>>,        // topic -> message
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            subscriptions: RwLock::new(HashMap::new()),
            inflight: RwLock::new(HashMap::new()),
            retained: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage for InMemoryStorage {
    // ========== Session Operations ==========

    fn save_session(&self, session: &PersistedSession) -> StorageResult<()> {
        let mut sessions = self.sessions.write().unwrap();
        sessions.insert(session.client_id.clone(), session.clone());
        Ok(())
    }

    fn load_session(&self, client_id: &str) -> StorageResult<Option<PersistedSession>> {
        let sessions = self.sessions.read().unwrap();
        Ok(sessions.get(client_id).cloned())
    }

    fn delete_session(&self, client_id: &str) -> StorageResult<()> {
        let mut sessions = self.sessions.write().unwrap();
        let mut subscriptions = self.subscriptions.write().unwrap();
        let mut inflight = self.inflight.write().unwrap();

        sessions.remove(client_id);
        subscriptions.remove(client_id);
        inflight.remove(client_id);

        Ok(())
    }

    fn session_exists(&self, client_id: &str) -> StorageResult<bool> {
        let sessions = self.sessions.read().unwrap();
        Ok(sessions.contains_key(client_id))
    }

    fn delete_expired_sessions(&self, older_than: DateTime<Utc>) -> StorageResult<usize> {
        let mut sessions = self.sessions.write().unwrap();
        let mut subscriptions = self.subscriptions.write().unwrap();
        let mut inflight = self.inflight.write().unwrap();

        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.updated_at < older_than)
            .map(|(k, _)| k.clone())
            .collect();

        let count = expired.len();

        for client_id in expired {
            sessions.remove(&client_id);
            subscriptions.remove(&client_id);
            inflight.remove(&client_id);
        }

        Ok(count)
    }

    // ========== Subscription Operations ==========

    fn save_subscription(&self, sub: &PersistedSubscription) -> StorageResult<()> {
        let mut subscriptions = self.subscriptions.write().unwrap();
        let subs = subscriptions.entry(sub.client_id.clone()).or_default();

        // Update existing or add new
        if let Some(existing) = subs.iter_mut().find(|s| s.topic_filter == sub.topic_filter) {
            existing.qos = sub.qos;
        } else {
            subs.push(sub.clone());
        }

        Ok(())
    }

    fn save_subscriptions(&self, subscriptions: &[PersistedSubscription]) -> StorageResult<()> {
        for sub in subscriptions {
            self.save_subscription(sub)?;
        }
        Ok(())
    }

    fn load_subscriptions(&self, client_id: &str) -> StorageResult<Vec<PersistedSubscription>> {
        let subscriptions = self.subscriptions.read().unwrap();
        Ok(subscriptions.get(client_id).cloned().unwrap_or_default())
    }

    fn delete_subscription(&self, client_id: &str, topic_filter: &str) -> StorageResult<()> {
        let mut subscriptions = self.subscriptions.write().unwrap();
        if let Some(subs) = subscriptions.get_mut(client_id) {
            subs.retain(|s| s.topic_filter != topic_filter);
        }
        Ok(())
    }

    fn delete_all_subscriptions(&self, client_id: &str) -> StorageResult<()> {
        let mut subscriptions = self.subscriptions.write().unwrap();
        subscriptions.remove(client_id);
        Ok(())
    }

    // ========== In-flight Message Operations ==========

    fn save_inflight_message(&self, msg: &PersistedInflightMessage) -> StorageResult<()> {
        let mut inflight = self.inflight.write().unwrap();
        let messages = inflight.entry(msg.client_id.clone()).or_default();

        // Update existing or add new
        if let Some(existing) = messages.iter_mut().find(|m| m.packet_id == msg.packet_id) {
            existing.retry_count = msg.retry_count;
            existing.created_at = msg.created_at;
        } else {
            messages.push(msg.clone());
        }

        Ok(())
    }

    fn load_inflight_messages(
        &self,
        client_id: &str,
    ) -> StorageResult<Vec<PersistedInflightMessage>> {
        let inflight = self.inflight.read().unwrap();
        Ok(inflight.get(client_id).cloned().unwrap_or_default())
    }

    fn delete_inflight_message(&self, client_id: &str, packet_id: u16) -> StorageResult<()> {
        let mut inflight = self.inflight.write().unwrap();
        if let Some(messages) = inflight.get_mut(client_id) {
            messages.retain(|m| m.packet_id != packet_id);
        }
        Ok(())
    }

    fn delete_all_inflight_messages(&self, client_id: &str) -> StorageResult<()> {
        let mut inflight = self.inflight.write().unwrap();
        inflight.remove(client_id);
        Ok(())
    }

    // ========== Retained Message Operations ==========

    fn save_retained_message(&self, msg: &PersistedRetainedMessage) -> StorageResult<()> {
        let mut retained = self.retained.write().unwrap();
        retained.insert(msg.topic.clone(), msg.clone());
        Ok(())
    }

    fn load_retained_message(
        &self,
        topic: &str,
    ) -> StorageResult<Option<PersistedRetainedMessage>> {
        let retained = self.retained.read().unwrap();
        Ok(retained.get(topic).cloned())
    }

    fn load_all_retained_messages(&self) -> StorageResult<Vec<PersistedRetainedMessage>> {
        let retained = self.retained.read().unwrap();
        Ok(retained.values().cloned().collect())
    }

    fn delete_retained_message(&self, topic: &str) -> StorageResult<()> {
        let mut retained = self.retained.write().unwrap();
        retained.remove(topic);
        Ok(())
    }

    fn count_retained_messages(&self) -> StorageResult<usize> {
        let retained = self.retained.read().unwrap();
        Ok(retained.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::StoredQoS;
    use bytes::Bytes;

    #[test]
    fn test_session_crud() {
        let storage = InMemoryStorage::new();
        let now = Utc::now();

        let session = PersistedSession {
            client_id: "test-client".to_string(),
            next_packet_id: 100,
            keep_alive: 60,
            will_message: None,
            created_at: now,
            updated_at: now,
        };

        storage.save_session(&session).unwrap();

        let loaded = storage.load_session("test-client").unwrap().unwrap();
        assert_eq!(loaded.client_id, "test-client");
        assert_eq!(loaded.next_packet_id, 100);

        assert!(storage.session_exists("test-client").unwrap());
        assert!(!storage.session_exists("nonexistent").unwrap());

        storage.delete_session("test-client").unwrap();
        assert!(!storage.session_exists("test-client").unwrap());
    }

    #[test]
    fn test_subscription_crud() {
        let storage = InMemoryStorage::new();

        let sub = PersistedSubscription {
            client_id: "client1".to_string(),
            topic_filter: "test/#".to_string(),
            qos: StoredQoS::AtLeastOnce,
        };

        storage.save_subscription(&sub).unwrap();

        let loaded = storage.load_subscriptions("client1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].topic_filter, "test/#");

        storage.delete_subscription("client1", "test/#").unwrap();
        let loaded = storage.load_subscriptions("client1").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_retained_message_crud() {
        let storage = InMemoryStorage::new();
        let now = Utc::now();

        let msg = PersistedRetainedMessage {
            topic: "sensor/temp".to_string(),
            payload: Bytes::from("25.5"),
            qos: StoredQoS::AtMostOnce,
            updated_at: now,
        };

        storage.save_retained_message(&msg).unwrap();

        let loaded = storage
            .load_retained_message("sensor/temp")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.topic, "sensor/temp");
        assert_eq!(loaded.payload, Bytes::from("25.5"));

        assert_eq!(storage.count_retained_messages().unwrap(), 1);

        storage.delete_retained_message("sensor/temp").unwrap();
        assert!(storage
            .load_retained_message("sensor/temp")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_cascade_delete() {
        let storage = InMemoryStorage::new();
        let now = Utc::now();

        // Create session with subscriptions and inflight
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

        // Delete session should cascade
        storage.delete_session("client1").unwrap();

        assert!(!storage.session_exists("client1").unwrap());
        assert!(storage.load_subscriptions("client1").unwrap().is_empty());
        assert!(storage
            .load_inflight_messages("client1")
            .unwrap()
            .is_empty());
    }
}
