use crate::protocol::packet::QoS;
use crate::protocol::v3::subscribe_return_codes::MAXIMUM_QOS_0;
use crate::protocol::{Packet, PublishPacket};
use crate::session::Mailbox;
use std::collections::{HashSet, HashMap};
use tokio::sync::RwLock;
use tracing::info;

struct RouterInternal {
    // FILTER -> (SESSION_ID -> SENDER)
    filters: HashMap<String, HashMap<String, Mailbox>>,
    // SESSION_ID -> FILTER
    sessions: HashMap<String, HashSet<String>>,
}

pub struct Router {
    data: RwLock<RouterInternal>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(RouterInternal {
                filters: HashMap::new(),
                sessions: HashMap::new(),
            }),
        }
    }

    pub async fn subscribe(&self, session_id: &str, sender: Mailbox, topic_filters: &Vec<(String, QoS)>) -> Vec<u8> {
        info!("Session {} subscribing to {} topics", session_id, topic_filters.len());

        let mut return_codes = Vec::new();
        let mut router = self.data.write().await;

        for filter in topic_filters.iter() {
            // Store filters -> (session_id -> sender)
            router.filters.entry(filter.0.to_string()).or_insert(HashMap::new())
                .insert(session_id.to_owned(), sender.clone());
            // Store session_id -> filter
            router.sessions.entry(session_id.to_owned()).or_insert(HashSet::new())
                .insert(filter.0.to_owned());
            return_codes.push(MAXIMUM_QOS_0);
            info!("SUBSCRIBE session_id: {}, filter: {}", session_id, filter.0);
        }

        return_codes
    }

    pub async fn unsubscribe(&self, session_id: &str, topic_filters: &Vec<String>) {
        info!("Session {} unsubscribing to {} topics", session_id, topic_filters.len());
        let mut router = self.data.write().await;

        for filter in topic_filters.iter() {
            if let Some(record) = router.filters.get_mut(filter) {
                record.remove(session_id);
            }
            if let Some(record) = router.sessions.get_mut(session_id) {
                record.remove(filter);
            }
        }
    }

    pub async fn unsubscribe_all(&self, session_id: &str) {
        info!("Unsubscribed {} from all topics", session_id);
        let mut router = self.data.write().await;
        if let Some(filters) = router.sessions.remove(session_id) {
            for filter in filters.iter() {
                if let Some(record) = router.filters.get_mut(filter) {
                    record.remove(session_id);
                }
            }
        }
    }

    pub async fn route(&self, topic: &str, payload: &[u8]) {
        let router = self.data.read().await;

        let packet = PublishPacket {
            topic: topic.to_string(),
            packet_id: None,
            payload: payload.to_vec().into(),
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
        };

        for (filter, sessions) in router.filters.iter() {
            if Self::topic_matches(topic, filter) {
                for (session_id, sender) in sessions.iter() {
                    info!("ROUTE topic:{} session_id:{}", topic, session_id);
                    sender.send(Packet::Publish(packet.clone())).await.unwrap_or_else(|e| {
                        info!("Route topic {} to session {} error: {}", topic, session_id, e);
                    });
                }
            }
        }
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
                // + should not match empty levels
                if topic_part.is_empty() {
                    false
                } else {
                    Self::match_parts(topic_parts, filter_parts, topic_idx + 1, filter_idx + 1)
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(Router::topic_matches("home/kitchen/temperature", "home/kitchen/temperature"));
        assert!(Router::topic_matches("a", "a"));
        assert!(Router::topic_matches("", ""));
    }

    #[test]
    fn test_exact_mismatch() {
        assert!(!Router::topic_matches("home/kitchen/temperature", "home/kitchen/humidity"));
        assert!(!Router::topic_matches("home/kitchen", "home/kitchen/temperature"));
        assert!(!Router::topic_matches("home/kitchen/temperature", "home/kitchen"));
        assert!(!Router::topic_matches("abc", "xyz"));
        assert!(!Router::topic_matches("a", ""));
        assert!(!Router::topic_matches("", "a"));
    }

    #[test]
    fn test_single_level_wildcard_plus() {
        // Basic + wildcard tests
        assert!(Router::topic_matches("home/kitchen/temperature", "home/+/temperature"));
        assert!(Router::topic_matches("home/bedroom/temperature", "home/+/temperature"));
        assert!(Router::topic_matches("home/livingroom/temperature", "home/+/temperature"));

        // + at beginning
        assert!(Router::topic_matches("kitchen/temperature", "+/temperature"));
        assert!(Router::topic_matches("bedroom/temperature", "+/temperature"));

        // + at end
        assert!(Router::topic_matches("home/kitchen", "home/+"));
        assert!(Router::topic_matches("home/bedroom", "home/+"));

        // Multiple + wildcards
        assert!(Router::topic_matches("home/kitchen/sensor/temperature", "home/+/sensor/+"));
        assert!(Router::topic_matches("office/meeting/sensor/humidity", "office/+/sensor/+"));

        // + should not match empty level
        assert!(!Router::topic_matches("home//temperature", "home/+/temperature"));
        assert!(!Router::topic_matches("home/temperature", "home/+/temperature"));

        // + should not match multiple levels
        assert!(!Router::topic_matches("home/kitchen/cabinet/temperature", "home/+/temperature"));
    }

    #[test]
    fn test_multi_level_wildcard_hash() {
        // Basic # wildcard tests
        assert!(Router::topic_matches("home/kitchen/temperature", "home/#"));
        assert!(Router::topic_matches("home/kitchen/sensor/temperature/celsius", "home/#"));
        assert!(Router::topic_matches("home", "home/#"));

        // # at root
        assert!(Router::topic_matches("home/kitchen/temperature", "#"));
        assert!(Router::topic_matches("office/meeting/room", "#"));
        assert!(Router::topic_matches("sensor", "#"));
        assert!(Router::topic_matches("a/b/c/d/e/f/g", "#"));

        // # should match zero levels
        assert!(Router::topic_matches("home", "home/#"));

        // # should match empty topic if filter is just #
        assert!(Router::topic_matches("", "#"));

        // # must be last in filter
        assert!(Router::topic_matches("home/kitchen/temperature", "home/kitchen/#"));
    }

    #[test]
    fn test_combined_wildcards() {
        // Combining + and # wildcards
        assert!(Router::topic_matches("home/kitchen/sensor/temperature", "home/+/#"));
        assert!(Router::topic_matches("home/bedroom/light/brightness", "home/+/#"));
        assert!(Router::topic_matches("home/kitchen", "home/+/#"));

        // Multiple + before #
        assert!(Router::topic_matches("building/floor2/room5/sensor/temp", "building/+/+/#"));
        assert!(Router::topic_matches("building/floor1/room3/light", "building/+/+/#"));

        // Edge case: + and # together
        assert!(Router::topic_matches("a/b/c/d", "+/+/#"));
        assert!(Router::topic_matches("x/y", "+/+/#"));
    }

    #[test]
    fn test_edge_cases() {
        // Empty topics and filters
        assert!(Router::topic_matches("", ""));
        assert!(Router::topic_matches("", "#"));
        assert!(!Router::topic_matches("a", ""));
        assert!(!Router::topic_matches("", "a"));

        // Single character topics
        assert!(Router::topic_matches("a", "a"));
        assert!(Router::topic_matches("a", "+"));
        assert!(Router::topic_matches("a", "#"));

        // Very long topics
        let long_topic = "a/".repeat(100) + "b";
        let long_filter = "+/".repeat(100) + "+";
        assert!(Router::topic_matches(&long_topic, &long_filter));
        assert!(Router::topic_matches(&long_topic, "#"));

        // Topics with special characters
        assert!(Router::topic_matches("home/kitchen-sensor/temp_1", "home/+/+"));
        assert!(Router::topic_matches("device@123/status", "+/status"));
        assert!(Router::topic_matches("sensor.temperature.celsius", "sensor.temperature.celsius"));

        // Unicode characters
        assert!(Router::topic_matches("家/厨房/温度", "家/+/温度"));
        assert!(Router::topic_matches("🏠/🔥/🌡️", "🏠/#"));
    }

    #[test]
    fn test_invalid_wildcard_patterns() {
        // These should not match because # is not at the end
        // Note: Our implementation should handle these gracefully
        assert!(!Router::topic_matches("home/kitchen/temp", "home/#/temp"));
        assert!(!Router::topic_matches("a/b/c", "#/b/c"));
    }

    #[test]
    fn test_mqtt_spec_examples() {
        // Examples from MQTT specification

        // sport/tennis/player1
        assert!(Router::topic_matches("sport/tennis/player1", "sport/tennis/player1"));
        assert!(Router::topic_matches("sport/tennis/player1", "sport/tennis/+"));
        assert!(!Router::topic_matches("sport/tennis/player1", "sport/+"));
        assert!(Router::topic_matches("sport/tennis/player1", "+/tennis/player1"));
        assert!(!Router::topic_matches("sport/tennis/player1", "+/+"));
        assert!(Router::topic_matches("sport/tennis/player1", "+/+/+"));
        assert!(Router::topic_matches("sport/tennis/player1", "#"));
        assert!(Router::topic_matches("sport/tennis/player1", "sport/#"));
        assert!(Router::topic_matches("sport/tennis/player1", "sport/tennis/#"));

        // These should NOT match
        assert!(!Router::topic_matches("sport/tennis/player1", "sport/tennis"));
        assert!(!Router::topic_matches("sport/tennis/player1", "tennis"));
        assert!(!Router::topic_matches("sport/tennis/player1", "sport/tennis/player1/ranking"));
        assert!(!Router::topic_matches("sport/tennis/player1", "+"));
        assert!(!Router::topic_matches("sport/tennis/player1", "+/+"));

        // sport/
        assert!(Router::topic_matches("sport/", "sport/"));
        assert!(!Router::topic_matches("sport/", "sport/+"));
        assert!(!Router::topic_matches("sport/", "+/+"));
        assert!(Router::topic_matches("sport/", "#"));
        assert!(Router::topic_matches("sport/", "sport/#"));

        // /finance
        assert!(Router::topic_matches("/finance", "/finance"));
        assert!(!Router::topic_matches("/finance", "+/finance")); // + should not match empty level
        assert!(!Router::topic_matches("/finance", "+/+"));
        assert!(Router::topic_matches("/finance", "#"));
        assert!(Router::topic_matches("/finance", "/#"));
    }

    #[test]
    fn test_stress_patterns() {
        // Deeply nested topics
        let deep_topic = (0..50).map(|i| format!("level{}", i)).collect::<Vec<_>>().join("/");
        let deep_plus_filter = (0..50).map(|_| "+").collect::<Vec<_>>().join("/");
        assert!(Router::topic_matches(&deep_topic, &deep_plus_filter));
        assert!(Router::topic_matches(&deep_topic, "#"));

        // Many consecutive slashes
        assert!(!Router::topic_matches("a///b", "a/+/+/b")); // + should not match empty levels
        assert!(!Router::topic_matches("a//", "a/+/+")); // + should not match empty levels  
        assert!(!Router::topic_matches("//a", "+/+/a")); // + should not match empty levels

        // Mixed wildcards stress test
        assert!(Router::topic_matches("a/b/c/d/e/f/g/h", "a/+/c/+/#"));
        assert!(Router::topic_matches("x/y/z", "+/+/#"));
        assert!(!Router::topic_matches("a/b", "a/+/c/+"));
    }

    #[test]
    fn test_boundary_conditions() {
        // Single level topics
        assert!(Router::topic_matches("a", "+"));
        assert!(Router::topic_matches("a", "#"));
        assert!(!Router::topic_matches("a/b", "+"));

        // Two level topics
        assert!(Router::topic_matches("a/b", "+/+"));
        assert!(Router::topic_matches("a/b", "#"));
        assert!(Router::topic_matches("a/b", "a/#"));
        assert!(Router::topic_matches("a/b", "+/#"));

        // Filter longer than topic
        assert!(!Router::topic_matches("a", "a/b"));
        assert!(!Router::topic_matches("a/b", "a/b/c"));
        assert!(!Router::topic_matches("a", "+/+"));

        // Topic longer than filter
        assert!(!Router::topic_matches("a/b/c", "a/b"));
        assert!(!Router::topic_matches("a/b", "a"));
        assert!(Router::topic_matches("a/b/c", "a/#"));
        assert!(Router::topic_matches("a/b/c", "#"));
    }
}
