use std::collections::HashMap;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::server::broker::Message;

#[derive(Debug)]
pub struct Session {
    pub client_id: String,
    pub clean_session: bool,
    pub keep_alive: u16,
    pub subscriptions: HashMap<String, u8>, // topic -> QoS
    pub message_receiver: Option<broadcast::Receiver<Message>>,
}

impl Session {
    pub fn new(client_id: String, clean_session: bool, keep_alive: u16) -> Self {
        let actual_client_id = if client_id.is_empty() {
            // Generate a unique client ID if none provided
            format!("auto-{}", Uuid::new_v4().simple())
        } else {
            client_id
        };

        Self {
            client_id: actual_client_id,
            clean_session,
            keep_alive,
            subscriptions: HashMap::new(),
            message_receiver: None,
        }
    }

    pub fn add_subscription(&mut self, topic: String, qos: u8) {
        self.subscriptions.insert(topic, qos);
    }

    pub fn remove_subscription(&mut self, topic: &str) {
        self.subscriptions.remove(topic);
    }

    pub fn set_message_receiver(&mut self, receiver: broadcast::Receiver<Message>) {
        self.message_receiver = Some(receiver);
    }
}