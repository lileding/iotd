use crate::storage::traits::{Storage, StorageResult};
use crate::storage::types::{
    PersistedInboundQos2Message, PersistedInflightMessage, PersistedRetainedMessage,
    PersistedSession, PersistedSubscription,
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
    inbound_qos2: RwLock<HashMap<String, Vec<PersistedInboundQos2Message>>>, // client_id -> messages
    retained: RwLock<HashMap<String, PersistedRetainedMessage>>,             // topic -> message
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            subscriptions: RwLock::new(HashMap::new()),
            inflight: RwLock::new(HashMap::new()),
            inbound_qos2: RwLock::new(HashMap::new()),
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

    fn save_session(
        &self,
        session: &PersistedSession,
        subscriptions: &[PersistedSubscription],
        inflight: &[PersistedInflightMessage],
        inbound_qos2: &[PersistedInboundQos2Message],
    ) -> StorageResult<()> {
        // Acquire all locks to ensure atomicity
        let mut sessions_guard = self.sessions.write().unwrap();
        let mut subs_guard = self.subscriptions.write().unwrap();
        let mut inflight_guard = self.inflight.write().unwrap();
        let mut inbound_qos2_guard = self.inbound_qos2.write().unwrap();

        let client_id = &session.client_id;

        // Save session
        sessions_guard.insert(client_id.clone(), session.clone());

        // Replace subscriptions
        subs_guard.insert(client_id.clone(), subscriptions.to_vec());

        // Replace in-flight messages
        inflight_guard.insert(client_id.clone(), inflight.to_vec());

        // Replace inbound QoS=2 messages
        inbound_qos2_guard.insert(client_id.clone(), inbound_qos2.to_vec());

        Ok(())
    }

    fn load_session(&self, client_id: &str) -> StorageResult<Option<PersistedSession>> {
        let sessions = self.sessions.read().unwrap();
        Ok(sessions.get(client_id).cloned())
    }

    fn load_subscriptions(&self, client_id: &str) -> StorageResult<Vec<PersistedSubscription>> {
        let subscriptions = self.subscriptions.read().unwrap();
        Ok(subscriptions.get(client_id).cloned().unwrap_or_default())
    }

    fn load_inflight_messages(
        &self,
        client_id: &str,
    ) -> StorageResult<Vec<PersistedInflightMessage>> {
        let inflight = self.inflight.read().unwrap();
        Ok(inflight.get(client_id).cloned().unwrap_or_default())
    }

    fn load_inbound_qos2_messages(
        &self,
        client_id: &str,
    ) -> StorageResult<Vec<PersistedInboundQos2Message>> {
        let inbound_qos2 = self.inbound_qos2.read().unwrap();
        Ok(inbound_qos2.get(client_id).cloned().unwrap_or_default())
    }

    fn delete_session(&self, client_id: &str) -> StorageResult<()> {
        let mut sessions = self.sessions.write().unwrap();
        let mut subscriptions = self.subscriptions.write().unwrap();
        let mut inflight = self.inflight.write().unwrap();
        let mut inbound_qos2 = self.inbound_qos2.write().unwrap();

        sessions.remove(client_id);
        subscriptions.remove(client_id);
        inflight.remove(client_id);
        inbound_qos2.remove(client_id);

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
        let mut inbound_qos2 = self.inbound_qos2.write().unwrap();

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
            inbound_qos2.remove(&client_id);
        }

        Ok(count)
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
    fn test_session_save_and_load() {
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

        let subs = vec![PersistedSubscription {
            client_id: "test-client".to_string(),
            topic_filter: "test/#".to_string(),
            qos: StoredQoS::AtLeastOnce,
        }];

        let inflight = vec![PersistedInflightMessage {
            client_id: "test-client".to_string(),
            packet_id: 1,
            topic: "test/topic".to_string(),
            payload: Bytes::from("data"),
            qos: StoredQoS::AtLeastOnce,
            retain: false,
            retry_count: 0,
            qos2_state: None,
            created_at: now,
        }];

        storage
            .save_session(&session, &subs, &inflight, &[])
            .unwrap();

        // Load and verify
        let loaded_session = storage.load_session("test-client").unwrap().unwrap();
        assert_eq!(loaded_session.client_id, "test-client");
        assert_eq!(loaded_session.next_packet_id, 100);

        let loaded_subs = storage.load_subscriptions("test-client").unwrap();
        assert_eq!(loaded_subs.len(), 1);
        assert_eq!(loaded_subs[0].topic_filter, "test/#");

        let loaded_inflight = storage.load_inflight_messages("test-client").unwrap();
        assert_eq!(loaded_inflight.len(), 1);
        assert_eq!(loaded_inflight[0].packet_id, 1);

        assert!(storage.session_exists("test-client").unwrap());
        assert!(!storage.session_exists("nonexistent").unwrap());
    }

    #[test]
    fn test_session_delete_cascades() {
        let storage = InMemoryStorage::new();
        let now = Utc::now();

        let session = PersistedSession {
            client_id: "client1".to_string(),
            next_packet_id: 1,
            keep_alive: 60,
            will_message: None,
            created_at: now,
            updated_at: now,
        };

        let subs = vec![PersistedSubscription {
            client_id: "client1".to_string(),
            topic_filter: "test/#".to_string(),
            qos: StoredQoS::AtLeastOnce,
        }];

        let inflight = vec![PersistedInflightMessage {
            client_id: "client1".to_string(),
            packet_id: 1,
            topic: "test/topic".to_string(),
            payload: Bytes::from("data"),
            qos: StoredQoS::AtLeastOnce,
            retain: false,
            retry_count: 0,
            qos2_state: None,
            created_at: now,
        }];

        storage
            .save_session(&session, &subs, &inflight, &[])
            .unwrap();

        // Delete session should cascade
        storage.delete_session("client1").unwrap();

        assert!(!storage.session_exists("client1").unwrap());
        assert!(storage.load_subscriptions("client1").unwrap().is_empty());
        assert!(storage
            .load_inflight_messages("client1")
            .unwrap()
            .is_empty());
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
}
