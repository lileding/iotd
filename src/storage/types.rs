use crate::protocol::packet::QoS;
use bytes::Bytes;
use chrono::{DateTime, Utc};

/// QoS level for persistence (mirrors packet::QoS but owns its data)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StoredQoS {
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

impl StoredQoS {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(StoredQoS::AtMostOnce),
            1 => Some(StoredQoS::AtLeastOnce),
            2 => Some(StoredQoS::ExactlyOnce),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl From<QoS> for StoredQoS {
    fn from(qos: QoS) -> Self {
        match qos {
            QoS::AtMostOnce => StoredQoS::AtMostOnce,
            QoS::AtLeastOnce => StoredQoS::AtLeastOnce,
            QoS::ExactlyOnce => StoredQoS::ExactlyOnce,
        }
    }
}

impl From<StoredQoS> for QoS {
    fn from(qos: StoredQoS) -> Self {
        match qos {
            StoredQoS::AtMostOnce => QoS::AtMostOnce,
            StoredQoS::AtLeastOnce => QoS::AtLeastOnce,
            StoredQoS::ExactlyOnce => QoS::ExactlyOnce,
        }
    }
}

/// Persisted session state for clean_session=false clients
#[derive(Debug, Clone)]
pub struct PersistedSession {
    pub client_id: String,
    pub next_packet_id: u16,
    pub keep_alive: u16,
    pub will_message: Option<PersistedWillMessage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Will message to be published on abnormal disconnect
#[derive(Debug, Clone)]
pub struct PersistedWillMessage {
    pub topic: String,
    pub payload: Bytes,
    pub qos: StoredQoS,
    pub retain: bool,
}

/// Subscription entry for a session
#[derive(Debug, Clone)]
pub struct PersistedSubscription {
    pub client_id: String,
    pub topic_filter: String,
    pub qos: StoredQoS,
}

/// QoS=2 state for persistence
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PersistedQos2State {
    AwaitingPubRec = 0,
    AwaitingPubComp = 1,
}

impl PersistedQos2State {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(PersistedQos2State::AwaitingPubRec),
            1 => Some(PersistedQos2State::AwaitingPubComp),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// In-flight QoS=1/2 message awaiting acknowledgement
#[derive(Debug, Clone)]
pub struct PersistedInflightMessage {
    pub client_id: String,
    pub packet_id: u16,
    pub topic: String,
    pub payload: Bytes,
    pub qos: StoredQoS,
    pub retain: bool,
    pub retry_count: u32,
    pub qos2_state: Option<PersistedQos2State>, // None for QoS=1, Some for QoS=2
    pub created_at: DateTime<Utc>,
}

/// Inbound QoS=2 message awaiting PUBREL
#[derive(Debug, Clone)]
pub struct PersistedInboundQos2Message {
    pub client_id: String,
    pub packet_id: u16,
    pub topic: String,
    pub payload: Bytes,
    pub retain: bool,
    pub received_at: DateTime<Utc>,
}

/// Retained message stored on a topic
#[derive(Debug, Clone)]
pub struct PersistedRetainedMessage {
    pub topic: String,
    pub payload: Bytes,
    pub qos: StoredQoS,
    pub updated_at: DateTime<Utc>,
}
