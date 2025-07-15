use crate::protocol::{PublishPacket, QoS};
use dashmap::DashMap;
use std::collections::HashSet;
use tokio::sync::broadcast;
use tracing::{debug, warn};

pub type ClientId = String;
pub type TopicFilter = String;

#[derive(Debug, Clone)]
pub struct Message {
    pub topic: String,
    pub payload: bytes::Bytes,
    pub qos: QoS,
    pub retain: bool,
}

pub struct Broker {
    subscriptions: DashMap<TopicFilter, HashSet<ClientId>>,
    clients: DashMap<ClientId, broadcast::Sender<Message>>,
    retained_messages: DashMap<String, Message>,
}

impl Broker {
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
            clients: DashMap::new(),
            retained_messages: DashMap::new(),
        }
    }

    pub fn register_client(&self, client_id: ClientId) -> broadcast::Receiver<Message> {
        let (tx, rx) = broadcast::channel(1000);
        self.clients.insert(client_id, tx);
        rx
    }

    pub fn unregister_client(&self, client_id: &str) {
        self.clients.remove(client_id);
        
        // Remove client from all subscriptions
        for mut subscription in self.subscriptions.iter_mut() {
            subscription.value_mut().remove(client_id);
        }
        
        // Clean up empty subscriptions
        self.subscriptions.retain(|_, clients| !clients.is_empty());
    }

    pub fn subscribe(&self, client_id: ClientId, topic_filter: TopicFilter) -> Vec<Message> {
        debug!("Client {} subscribing to {}", client_id, topic_filter);
        
        self.subscriptions
            .entry(topic_filter.clone())
            .or_insert_with(HashSet::new)
            .insert(client_id);

        // Return retained messages matching the topic filter
        self.get_retained_messages(&topic_filter)
    }

    pub fn unsubscribe(&self, client_id: &str, topic_filter: &str) {
        debug!("Client {} unsubscribing from {}", client_id, topic_filter);
        
        if let Some(mut subscription) = self.subscriptions.get_mut(topic_filter) {
            subscription.remove(client_id);
        }
    }

    pub fn publish(&self, packet: &PublishPacket) {
        let message = Message {
            topic: packet.topic.clone(),
            payload: packet.payload.clone(),
            qos: packet.qos.clone(),
            retain: packet.retain,
        };

        debug!("Publishing message to topic: {}", packet.topic);

        // Handle retained messages
        if packet.retain {
            if packet.payload.is_empty() {
                // Empty payload with retain flag removes the retained message
                self.retained_messages.remove(&packet.topic);
            } else {
                self.retained_messages.insert(packet.topic.clone(), message.clone());
            }
        }

        // Find matching subscriptions and send message
        let mut sent_count = 0;
        for subscription in self.subscriptions.iter() {
            if Self::topic_matches(subscription.key(), &packet.topic) {
                for client_id in subscription.value().iter() {
                    if let Some(client_sender) = self.clients.get(client_id) {
                        match client_sender.send(message.clone()) {
                            Ok(_) => sent_count += 1,
                            Err(_) => {
                                warn!("Failed to send message to client: {}", client_id);
                            }
                        }
                    }
                }
            }
        }

        debug!("Message sent to {} subscribers", sent_count);
    }

    pub fn get_retained_messages(&self, topic_filter: &str) -> Vec<Message> {
        self.retained_messages
            .iter()
            .filter(|entry| Self::topic_matches(topic_filter, entry.key()))
            .map(|entry| entry.value().clone())
            .collect()
    }

    fn topic_matches(filter: &str, topic: &str) -> bool {
        if filter == topic {
            return true;
        }

        let filter_parts: Vec<&str> = filter.split('/').collect();
        let topic_parts: Vec<&str> = topic.split('/').collect();

        Self::match_parts(&filter_parts, &topic_parts, 0, 0)
    }

    fn match_parts(filter_parts: &[&str], topic_parts: &[&str], fi: usize, ti: usize) -> bool {
        if fi >= filter_parts.len() {
            return ti >= topic_parts.len();
        }

        match filter_parts[fi] {
            "#" => true, // Multi-level wildcard matches everything
            "+" => {
                // Single-level wildcard
                if ti >= topic_parts.len() {
                    return false;
                }
                Self::match_parts(filter_parts, topic_parts, fi + 1, ti + 1)
            }
            part => {
                if ti >= topic_parts.len() || part != topic_parts[ti] {
                    return false;
                }
                Self::match_parts(filter_parts, topic_parts, fi + 1, ti + 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_matching() {
        assert!(Broker::topic_matches("sport/tennis/player1", "sport/tennis/player1"));
        assert!(Broker::topic_matches("sport/tennis/+", "sport/tennis/player1"));
        assert!(Broker::topic_matches("sport/+", "sport/tennis"));
        assert!(Broker::topic_matches("sport/#", "sport/tennis/player1"));
        assert!(Broker::topic_matches("#", "sport/tennis/player1"));
        
        assert!(!Broker::topic_matches("sport/tennis", "sport/tennis/player1"));
        assert!(!Broker::topic_matches("sport", "sport/tennis"));
        assert!(!Broker::topic_matches("sport/+/player1", "sport/tennis"));
    }
}