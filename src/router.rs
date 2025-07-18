use crate::protocol::packet::QoS;
use crate::protocol::PublishPacket;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use std::sync::Arc;
use anyhow::Result;
use tracing::info;

pub struct Router {
    topic_filters: Mutex<HashMap<String, Vec<String>>>,
    subscribers: Mutex<HashMap<String, Arc<mpsc::Sender<PublishPacket>>>>,
    // Map of topic -> map of session_id -> (qos, sender)
    //subscriptions: RwLock<HashMap<String, HashMap<String, (QoS, mpsc::Sender<bytes::Bytes>)>>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            topic_filters: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(HashMap::new()),
            //subscriptions: RwLock::new(HashMap::new()),
        }
    }

    pub async fn subscribe(&self, session_id: &str, sender: Arc<mpsc::Sender<PublishPacket>>, topic_filters: &Vec<(String, QoS)>) -> Result<Vec<u8>> {
        let mut subscribers = self.subscribers.lock().await;
        
        info!("Session {} subscribing to {} topics", session_id, topic_filters.len());
        
        // Store the sender for this session
        subscribers.insert(session_id.to_string(), sender);
        let mut filters = self.topic_filters.lock().await;
        for topic in topic_filters.iter() {
            let sessions = filters.entry(topic.0.to_string()).or_insert(Vec::new());
            sessions.push(session_id.to_string());
            info!("SUBSCRIBE session_id: {}, topic: {}", session_id, topic.0); // FIXME
        }
        
        // Return QoS 0 for all topic filters (maximum QoS we support)
        let return_codes = vec![crate::protocol::v3::subscribe_return_codes::MAXIMUM_QOS_0; topic_filters.len()];
        Ok(return_codes)
    }

    pub async fn unsubscribe(&self, topic_filters: &Vec<String>, session_id: &str) -> Result<()> {
        Ok(())
    }

    pub async fn route(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let subscribers = self.subscribers.lock().await;

        let packet = PublishPacket {
            topic: topic.to_string(),
            packet_id: None,
            payload: payload.to_vec().into(),
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
        };
        if let Some(sessions) = self.topic_filters.lock().await.get(topic) {
            for session_id in sessions.iter() {
                if let Some(sender) = subscribers.get(session_id) {
                    info!("ROUTE topic:{} session_id:{}", topic, session_id); // FIXME
                    sender.send(packet.clone()).await.unwrap();
                }
            }
        }
        /*
        info!("Routing message to topic: {} to {} subscribers", topic, subscribers.len());
        
        // For now, send to all subscribers (no topic filtering yet)
        let mut delivered_count = 0;
        for (session_id, sender) in subscribers.iter() {
            // Create PUBLISH packet bytes
            let publish_bytes = Self::create_publish_packet(topic, payload, QoS::AtMostOnce);
            
            // Send message (non-blocking)
            if sender.try_send(publish_bytes).is_ok() {
                delivered_count += 1;
                info!("Delivered message to session {}", session_id);
            } else {
                info!("Failed to deliver message to session {}", session_id);
            }
        }
        
        info!("Message delivered to {} sessions", delivered_count);
        */
        Ok(())
    }

    pub async fn unsubscribe_all(&self, session_id: &str) {
        info!("Unsubscribed {} from all topics", session_id);
    }

    // Helper function to create PUBLISH packet bytes
    fn create_publish_packet(topic: &str, payload: &[u8], qos: QoS) -> bytes::Bytes {
        use bytes::BytesMut;
        use crate::protocol::packet::{Packet, PublishPacket};
        
        let publish_packet = PublishPacket {
            topic: topic.to_string(),
            qos,
            retain: false,
            dup: false,
            payload: payload.to_vec().into(),
            packet_id: None, // For QoS 0, no packet ID needed
        };
        
        let mut buf = BytesMut::new();
        Packet::Publish(publish_packet).encode(&mut buf);
        buf.freeze()
    }

    // Simple topic matching - supports + (single level) and # (multi level) wildcards
    fn topic_matches(topic: &str, filter: &str) -> bool {
        if topic == filter {
            return true;
        }
        
        let topic_parts: Vec<&str> = topic.split('/').collect();
        let filter_parts: Vec<&str> = filter.split('/').collect();
        
        Self::match_parts(&topic_parts, &filter_parts, 0, 0)
    }
    
    fn match_parts(topic_parts: &[&str], filter_parts: &[&str], topic_idx: usize, filter_idx: usize) -> bool {
        // If we've consumed all filter parts
        if filter_idx >= filter_parts.len() {
            return topic_idx >= topic_parts.len();
        }
        
        // If we've consumed all topic parts
        if topic_idx >= topic_parts.len() {
            // Only match if remaining filter parts are all "#"
            return filter_idx == filter_parts.len() - 1 && filter_parts[filter_idx] == "#";
        }
        
        let filter_part = filter_parts[filter_idx];
        let topic_part = topic_parts[topic_idx];
        
        match filter_part {
            "#" => {
                // Multi-level wildcard - matches everything remaining
                // Must be the last part of the filter
                filter_idx == filter_parts.len() - 1
            },
            "+" => {
                // Single-level wildcard - matches exactly one level
                Self::match_parts(topic_parts, filter_parts, topic_idx + 1, filter_idx + 1)
            },
            _ => {
                // Literal match
                if topic_part == filter_part {
                    Self::match_parts(topic_parts, filter_parts, topic_idx + 1, filter_idx + 1)
                } else {
                    false
                }
            }
        }
    }
}
