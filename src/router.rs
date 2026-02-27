use crate::protocol::packet::{PublishPacket, QoS};
use crate::protocol::v3::subscribe_return_codes::FAILURE;
use crate::protocol::Packet;
use crate::session::Mailbox;
use crate::storage::{PersistedRetainedMessage, Storage, StoredQoS};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maximum QoS level supported by the server
const MAX_SUPPORTED_QOS: u8 = 2;

/// Calculate effective QoS as minimum of two QoS levels, capped at server max
fn min_qos(a: QoS, b: QoS) -> QoS {
    let min_val = std::cmp::min(a as u8, b as u8);
    let capped = std::cmp::min(min_val, MAX_SUPPORTED_QOS);
    match capped {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::ExactlyOnce, // Cap at QoS 2
    }
}

struct RouterInternal {
    // FILTER -> (SESSION_ID -> (SENDER, QoS))
    filters: HashMap<String, HashMap<String, (Mailbox, QoS)>>,
    // SESSION_ID -> FILTER
    sessions: HashMap<String, HashSet<String>>,
}

pub struct Router {
    data: RwLock<RouterInternal>,
    storage: Arc<dyn Storage>,
    retained_message_limit: usize,
}

impl Router {
    pub fn new(retained_message_limit: usize, storage: Arc<dyn Storage>) -> Self {
        Self {
            data: RwLock::new(RouterInternal {
                filters: HashMap::new(),
                sessions: HashMap::new(),
            }),
            storage,
            retained_message_limit,
        }
    }

    /// Validates a topic name for PUBLISH packets (MQTT v3.1.1)
    fn validate_publish_topic(topic: &str) -> bool {
        // Topic name must not be empty
        if topic.is_empty() {
            return false;
        }

        // Topic name must not contain null characters
        if topic.contains('\0') {
            return false;
        }

        // Topic name must be valid UTF-8 (already guaranteed by Rust String)
        // Maximum topic length is 65535 bytes (handled by packet decoder)

        true
    }

    /// Check if a topic filter contains wildcards (+ or #)
    fn has_wildcards(filter: &str) -> bool {
        filter.contains('+') || filter.contains('#')
    }

    /// Validates a topic filter for SUBSCRIBE packets (supports wildcards)
    fn validate_subscribe_filter(filter: &str) -> bool {
        // Topic filter must not be empty
        if filter.is_empty() {
            return false;
        }

        // Topic filter must not contain null characters
        if filter.contains('\0') {
            return false;
        }

        // Validate wildcard usage
        let levels: Vec<&str> = filter.split('/').collect();
        for (i, level) in levels.iter().enumerate() {
            // Multi-level wildcard # must be last character
            if level.contains('#') && (*level != "#" || i != levels.len() - 1) {
                return false;
            }
            // Single-level wildcard + must be whole level
            if level.contains('+') && *level != "+" {
                return false;
            }
        }

        true
    }

    pub async fn subscribe(
        &self,
        session_id: &str,
        sender: Mailbox,
        topic_filters: &[(String, QoS)],
    ) -> (Vec<u8>, Vec<PublishPacket>) {
        info!(
            "Session {} subscribing to {} topics",
            session_id,
            topic_filters.len()
        );

        let mut return_codes = Vec::new();
        let mut retained_to_send = Vec::new();

        // Lazy-load all retained messages only if needed for wildcard matching
        let mut all_retained: Option<Vec<PersistedRetainedMessage>> = None;

        {
            let mut router = self.data.write().await;

            for filter in topic_filters.iter() {
                // Validate topic filter
                if !Self::validate_subscribe_filter(&filter.0) {
                    info!("Invalid topic filter: {}", filter.0);
                    return_codes.push(FAILURE);
                    continue;
                }

                // Store filters -> (session_id -> (sender, qos))
                router
                    .filters
                    .entry(filter.0.to_string())
                    .or_default()
                    .insert(session_id.to_owned(), (sender.clone(), filter.1));
                // Store session_id -> filter
                router
                    .sessions
                    .entry(session_id.to_owned())
                    .or_default()
                    .insert(filter.0.to_owned());

                // Grant minimum of requested QoS and server's maximum supported QoS
                let granted_qos = std::cmp::min(filter.1 as u8, MAX_SUPPORTED_QOS);
                return_codes.push(granted_qos);
                info!(
                    "SUBSCRIBE session_id: {}, filter: {}, granted_qos: {}",
                    session_id, filter.0, granted_qos
                );

                // Find retained messages matching this subscription
                self.find_retained_for_filter(
                    &filter.0,
                    filter.1,
                    &mut retained_to_send,
                    &mut all_retained,
                );
            }
        }

        (return_codes, retained_to_send)
    }

    /// Find retained messages matching a subscription filter
    fn find_retained_for_filter(
        &self,
        filter: &str,
        subscription_qos: QoS,
        retained_to_send: &mut Vec<PublishPacket>,
        all_retained: &mut Option<Vec<PersistedRetainedMessage>>,
    ) {
        if Self::has_wildcards(filter) {
            // Wildcard filter: need to check all retained messages
            // Lazy-load all retained messages on first wildcard filter
            if all_retained.is_none() {
                *all_retained = match self.storage.load_all_retained_messages() {
                    Ok(msgs) => Some(msgs),
                    Err(e) => {
                        warn!("Failed to load retained messages: {}", e);
                        Some(Vec::new())
                    }
                };
            }

            if let Some(messages) = all_retained {
                for retained in messages {
                    if Self::topic_matches(&retained.topic, filter) {
                        self.add_retained_packet(
                            retained,
                            subscription_qos,
                            filter,
                            retained_to_send,
                        );
                    }
                }
            }
        } else {
            // Exact filter: direct lookup is more efficient
            match self.storage.load_retained_message(filter) {
                Ok(Some(retained)) => {
                    self.add_retained_packet(&retained, subscription_qos, filter, retained_to_send);
                }
                Ok(None) => {} // No retained message for this topic
                Err(e) => {
                    warn!("Failed to load retained message for {}: {}", filter, e);
                }
            }
        }
    }

    /// Create and add a retained message packet to the send list
    fn add_retained_packet(
        &self,
        retained: &PersistedRetainedMessage,
        subscription_qos: QoS,
        filter: &str,
        retained_to_send: &mut Vec<PublishPacket>,
    ) {
        let retained_qos = QoS::from(retained.qos);
        let effective_qos = min_qos(retained_qos, subscription_qos);

        let packet = PublishPacket {
            topic: retained.topic.clone(),
            packet_id: None,
            payload: retained.payload.clone(),
            qos: effective_qos,
            retain: true,
            dup: false,
        };
        retained_to_send.push(packet);
        debug!(
            "Found retained message for topic {} matching filter {}, QoS {:?} -> {:?}",
            retained.topic, filter, retained_qos, effective_qos
        );
    }

    pub async fn unsubscribe(&self, session_id: &str, topic_filters: &[String]) {
        info!(
            "Session {} unsubscribing to {} topics",
            session_id,
            topic_filters.len()
        );
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

    /// Get all subscriptions for a session (used for persistence)
    pub async fn get_subscriptions(&self, session_id: &str) -> Vec<(String, QoS)> {
        let router = self.data.read().await;
        let mut result = Vec::new();

        if let Some(filters) = router.sessions.get(session_id) {
            for filter in filters {
                if let Some(sessions) = router.filters.get(filter) {
                    if let Some((_, qos)) = sessions.get(session_id) {
                        result.push((filter.clone(), *qos));
                    }
                }
            }
        }

        result
    }

    pub async fn route(&self, packet: PublishPacket) {
        // Validate topic name
        if !Self::validate_publish_topic(&packet.topic) {
            info!("Invalid topic name in PUBLISH: {}", packet.topic);
            return;
        }

        // Handle retained message storage
        if packet.retain {
            self.handle_retained_message(&packet);
        }

        // Route to current subscribers
        let router = self.data.read().await;

        for (filter, sessions) in router.filters.iter() {
            if Self::topic_matches(&packet.topic, filter) {
                for (session_id, (sender, subscription_qos)) in sessions.iter() {
                    let effective_qos = min_qos(packet.qos, *subscription_qos);

                    // Create a packet for forwarding with appropriate QoS
                    // - Retain flag should be false when forwarding to existing subscribers
                    // - Packet ID should be None - each session will assign its own
                    let forward_packet = PublishPacket {
                        topic: packet.topic.clone(),
                        packet_id: None,
                        payload: packet.payload.clone(),
                        qos: effective_qos,
                        retain: false,
                        dup: false,
                    };

                    debug!(
                        "ROUTE topic:{} session_id:{} pub_qos:{:?} sub_qos:{:?} effective_qos:{:?}",
                        packet.topic, session_id, packet.qos, subscription_qos, effective_qos
                    );
                    sender
                        .send(Packet::Publish(forward_packet))
                        .await
                        .unwrap_or_else(|e| {
                            warn!(
                                "Route topic {} to session {} error: {}",
                                packet.topic, session_id, e
                            );
                        });
                }
            }
        }
    }

    /// Handle retained message storage (save or delete)
    fn handle_retained_message(&self, packet: &PublishPacket) {
        if packet.payload.is_empty() {
            // Empty payload with retain=true means delete the retained message
            if let Err(e) = self.storage.delete_retained_message(&packet.topic) {
                warn!(
                    "Failed to delete retained message for {}: {}",
                    packet.topic, e
                );
            } else {
                info!("Deleted retained message for topic: {}", packet.topic);
            }
        } else {
            // Check if we're at the limit and this is a new topic
            let existing = self.storage.load_retained_message(&packet.topic);
            let count = self.storage.count_retained_messages().unwrap_or(0);

            let is_new = matches!(existing, Ok(None));
            if is_new && count >= self.retained_message_limit {
                info!(
                    "Retained message limit reached ({}/{}), dropping message for topic: {}",
                    count, self.retained_message_limit, packet.topic
                );
            } else {
                // Store the retained message
                let retained = PersistedRetainedMessage {
                    topic: packet.topic.clone(),
                    payload: packet.payload.clone(),
                    qos: StoredQoS::from(packet.qos),
                    updated_at: Utc::now(),
                };
                if let Err(e) = self.storage.save_retained_message(&retained) {
                    warn!(
                        "Failed to save retained message for {}: {}",
                        packet.topic, e
                    );
                } else {
                    let new_count = self.storage.count_retained_messages().unwrap_or(0);
                    info!(
                        "Stored retained message for topic: {} (total: {}/{})",
                        packet.topic, new_count, self.retained_message_limit
                    );
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

    fn match_parts(
        topic_parts: &[&str],
        filter_parts: &[&str],
        topic_idx: usize,
        filter_idx: usize,
    ) -> bool {
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
            }
            "+" => {
                // Single-level wildcard - matches exactly one level
                // + should not match empty levels
                if topic_part.is_empty() {
                    false
                } else {
                    Self::match_parts(topic_parts, filter_parts, topic_idx + 1, filter_idx + 1)
                }
            }
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
        assert!(Router::topic_matches(
            "home/kitchen/temperature",
            "home/kitchen/temperature"
        ));
        assert!(Router::topic_matches("a", "a"));
        assert!(Router::topic_matches("", ""));
    }

    #[test]
    fn test_exact_mismatch() {
        assert!(!Router::topic_matches(
            "home/kitchen/temperature",
            "home/kitchen/humidity"
        ));
        assert!(!Router::topic_matches(
            "home/kitchen",
            "home/kitchen/temperature"
        ));
        assert!(!Router::topic_matches(
            "home/kitchen/temperature",
            "home/kitchen"
        ));
        assert!(!Router::topic_matches("abc", "xyz"));
        assert!(!Router::topic_matches("a", ""));
        assert!(!Router::topic_matches("", "a"));
    }

    #[test]
    fn test_single_level_wildcard_plus() {
        // Basic + wildcard tests
        assert!(Router::topic_matches(
            "home/kitchen/temperature",
            "home/+/temperature"
        ));
        assert!(Router::topic_matches(
            "home/bedroom/temperature",
            "home/+/temperature"
        ));
        assert!(Router::topic_matches(
            "home/livingroom/temperature",
            "home/+/temperature"
        ));

        // + at beginning
        assert!(Router::topic_matches(
            "kitchen/temperature",
            "+/temperature"
        ));
        assert!(Router::topic_matches(
            "bedroom/temperature",
            "+/temperature"
        ));

        // + at end
        assert!(Router::topic_matches("home/kitchen", "home/+"));
        assert!(Router::topic_matches("home/bedroom", "home/+"));

        // Multiple + wildcards
        assert!(Router::topic_matches(
            "home/kitchen/sensor/temperature",
            "home/+/sensor/+"
        ));
        assert!(Router::topic_matches(
            "office/meeting/sensor/humidity",
            "office/+/sensor/+"
        ));

        // + should not match empty level
        assert!(!Router::topic_matches(
            "home//temperature",
            "home/+/temperature"
        ));
        assert!(!Router::topic_matches(
            "home/temperature",
            "home/+/temperature"
        ));

        // + should not match multiple levels
        assert!(!Router::topic_matches(
            "home/kitchen/cabinet/temperature",
            "home/+/temperature"
        ));
    }

    #[test]
    fn test_multi_level_wildcard_hash() {
        // Basic # wildcard tests
        assert!(Router::topic_matches("home/kitchen/temperature", "home/#"));
        assert!(Router::topic_matches(
            "home/kitchen/sensor/temperature/celsius",
            "home/#"
        ));
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
        assert!(Router::topic_matches(
            "home/kitchen/temperature",
            "home/kitchen/#"
        ));
    }

    #[test]
    fn test_combined_wildcards() {
        // Combining + and # wildcards
        assert!(Router::topic_matches(
            "home/kitchen/sensor/temperature",
            "home/+/#"
        ));
        assert!(Router::topic_matches(
            "home/bedroom/light/brightness",
            "home/+/#"
        ));
        assert!(Router::topic_matches("home/kitchen", "home/+/#"));

        // Multiple + before #
        assert!(Router::topic_matches(
            "building/floor2/room5/sensor/temp",
            "building/+/+/#"
        ));
        assert!(Router::topic_matches(
            "building/floor1/room3/light",
            "building/+/+/#"
        ));

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
        assert!(Router::topic_matches(
            "home/kitchen-sensor/temp_1",
            "home/+/+"
        ));
        assert!(Router::topic_matches("device@123/status", "+/status"));
        assert!(Router::topic_matches(
            "sensor.temperature.celsius",
            "sensor.temperature.celsius"
        ));

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
        assert!(Router::topic_matches(
            "sport/tennis/player1",
            "sport/tennis/player1"
        ));
        assert!(Router::topic_matches(
            "sport/tennis/player1",
            "sport/tennis/+"
        ));
        assert!(!Router::topic_matches("sport/tennis/player1", "sport/+"));
        assert!(Router::topic_matches(
            "sport/tennis/player1",
            "+/tennis/player1"
        ));
        assert!(!Router::topic_matches("sport/tennis/player1", "+/+"));
        assert!(Router::topic_matches("sport/tennis/player1", "+/+/+"));
        assert!(Router::topic_matches("sport/tennis/player1", "#"));
        assert!(Router::topic_matches("sport/tennis/player1", "sport/#"));
        assert!(Router::topic_matches(
            "sport/tennis/player1",
            "sport/tennis/#"
        ));

        // These should NOT match
        assert!(!Router::topic_matches(
            "sport/tennis/player1",
            "sport/tennis"
        ));
        assert!(!Router::topic_matches("sport/tennis/player1", "tennis"));
        assert!(!Router::topic_matches(
            "sport/tennis/player1",
            "sport/tennis/player1/ranking"
        ));
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
        let deep_topic = (0..50)
            .map(|i| format!("level{i}"))
            .collect::<Vec<_>>()
            .join("/");
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

    #[test]
    fn test_valid_topic_names() {
        // Valid topic names
        assert!(Router::validate_publish_topic("home/temperature"));
        assert!(Router::validate_publish_topic("a"));
        assert!(Router::validate_publish_topic("a/b/c/d/e/f"));
        assert!(Router::validate_publish_topic("/"));
        assert!(Router::validate_publish_topic("home/"));
        assert!(Router::validate_publish_topic("/home"));
        assert!(Router::validate_publish_topic("测试")); // UTF-8
        assert!(Router::validate_publish_topic("🚀")); // Emoji
        assert!(Router::validate_publish_topic(
            "home/kitchen/temperature/°C"
        ));

        // Topics can contain wildcards when publishing (they're just literal characters)
        assert!(Router::validate_publish_topic("home/+/temp"));
        assert!(Router::validate_publish_topic("home/#"));
    }

    #[test]
    fn test_invalid_topic_names() {
        // Empty topic
        assert!(!Router::validate_publish_topic(""));

        // Contains null character
        assert!(!Router::validate_publish_topic("home\0temperature"));
        assert!(!Router::validate_publish_topic("\0"));
        assert!(!Router::validate_publish_topic("home/\0/temp"));
    }

    #[test]
    fn test_valid_topic_filters() {
        // Valid filters without wildcards
        assert!(Router::validate_subscribe_filter("home/temperature"));
        assert!(Router::validate_subscribe_filter("a"));
        assert!(Router::validate_subscribe_filter("/"));
        assert!(Router::validate_subscribe_filter("home/"));
        assert!(Router::validate_subscribe_filter("/home"));

        // Valid single-level wildcards
        assert!(Router::validate_subscribe_filter("+"));
        assert!(Router::validate_subscribe_filter("home/+/temperature"));
        assert!(Router::validate_subscribe_filter("+/+/+"));
        assert!(Router::validate_subscribe_filter("home/+"));
        assert!(Router::validate_subscribe_filter("+/temperature"));

        // Valid multi-level wildcards
        assert!(Router::validate_subscribe_filter("#"));
        assert!(Router::validate_subscribe_filter("home/#"));
        assert!(Router::validate_subscribe_filter("home/kitchen/#"));
        assert!(Router::validate_subscribe_filter("+/#"));
        assert!(Router::validate_subscribe_filter("+/+/#"));

        // UTF-8 is valid
        assert!(Router::validate_subscribe_filter("测试/+/#"));
        assert!(Router::validate_subscribe_filter("🚀/#"));
    }

    #[test]
    fn test_invalid_topic_filters() {
        // Empty filter
        assert!(!Router::validate_subscribe_filter(""));

        // Contains null character
        assert!(!Router::validate_subscribe_filter("home\0temperature"));
        assert!(!Router::validate_subscribe_filter("\0"));
        assert!(!Router::validate_subscribe_filter("home/\0/temp"));

        // Invalid single-level wildcard usage
        assert!(!Router::validate_subscribe_filter("home/+a/temp")); // + not alone
        assert!(!Router::validate_subscribe_filter("home/a+/temp")); // + not alone
        assert!(!Router::validate_subscribe_filter("home/++/temp")); // + not alone
        assert!(!Router::validate_subscribe_filter("home/+abc/temp")); // + not alone

        // Invalid multi-level wildcard usage
        assert!(!Router::validate_subscribe_filter("home/#/temp")); // # must be last
        assert!(!Router::validate_subscribe_filter("home/a#")); // # not alone
        assert!(!Router::validate_subscribe_filter("home/#a")); // # not alone
        assert!(!Router::validate_subscribe_filter("home/##")); // # not alone
        assert!(!Router::validate_subscribe_filter("#/home")); // # must be last
        assert!(!Router::validate_subscribe_filter("home/kitchen/#/room")); // # must be last
    }

    #[test]
    fn test_edge_case_validation() {
        // Just slashes
        assert!(Router::validate_publish_topic("/"));
        assert!(Router::validate_publish_topic("//"));
        assert!(Router::validate_publish_topic("///"));
        assert!(Router::validate_subscribe_filter("/"));
        assert!(Router::validate_subscribe_filter("//"));
        assert!(Router::validate_subscribe_filter("///"));

        // Wildcards at boundaries
        assert!(Router::validate_subscribe_filter("/+"));
        assert!(Router::validate_subscribe_filter("+/"));
        assert!(Router::validate_subscribe_filter("/+/"));
        assert!(Router::validate_subscribe_filter("/#"));
        assert!(Router::validate_subscribe_filter("/+/#"));

        // Multiple # in different levels (still invalid)
        assert!(!Router::validate_subscribe_filter("#/#"));
        assert!(!Router::validate_subscribe_filter("home/#/#"));

        // Mixed valid and invalid patterns
        assert!(Router::validate_subscribe_filter("home/+/kitchen/+/sensor"));
        assert!(!Router::validate_subscribe_filter(
            "home/+/kitchen/#/sensor"
        ));
        assert!(!Router::validate_subscribe_filter(
            "home/+a/kitchen/+/sensor"
        ));
    }
}
