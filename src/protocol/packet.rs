use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::{self, Cursor};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug, Error)]
pub enum PacketError {
    #[error("Invalid packet type: {0}")]
    InvalidPacketType(u8),
    #[error("Invalid remaining length")]
    InvalidRemainingLength,
    #[error("Incomplete packet")]
    IncompletePacket,
    #[error("Invalid UTF-8 string")]
    InvalidUtf8,
    #[error("Invalid packet flags")]
    InvalidPacketFlags,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PacketType {
    Connect = 1,
    ConnAck = 2,
    Publish = 3,
    PubAck = 4,
    PubRec = 5,
    PubRel = 6,
    PubComp = 7,
    Subscribe = 8,
    SubAck = 9,
    Unsubscribe = 10,
    UnsubAck = 11,
    PingReq = 12,
    PingResp = 13,
    Disconnect = 14,
}

impl TryFrom<u8> for PacketType {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(PacketType::Connect),
            2 => Ok(PacketType::ConnAck),
            3 => Ok(PacketType::Publish),
            4 => Ok(PacketType::PubAck),
            5 => Ok(PacketType::PubRec),
            6 => Ok(PacketType::PubRel),
            7 => Ok(PacketType::PubComp),
            8 => Ok(PacketType::Subscribe),
            9 => Ok(PacketType::SubAck),
            10 => Ok(PacketType::Unsubscribe),
            11 => Ok(PacketType::UnsubAck),
            12 => Ok(PacketType::PingReq),
            13 => Ok(PacketType::PingResp),
            14 => Ok(PacketType::Disconnect),
            _ => Err(PacketError::InvalidPacketType(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QoS {
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

#[derive(Debug, Clone)]
pub struct FixedHeader {
    pub packet_type: PacketType,
    pub flags: u8,
    pub remaining_length: u32,
}

#[derive(Debug, Clone)]
pub enum Packet {
    Connect(ConnectPacket),
    ConnAck(ConnAckPacket),
    Publish(PublishPacket),
    PubAck(PubAckPacket),
    Subscribe(SubscribePacket),
    SubAck(SubAckPacket),
    Unsubscribe(UnsubscribePacket),
    UnsubAck(UnsubAckPacket),
    PingReq,
    PingResp,
    Disconnect,
}

#[derive(Debug, Clone)]
pub struct ConnectPacket {
    pub protocol_name: String,
    pub protocol_level: u8,
    pub clean_session: bool,
    pub keep_alive: u16,
    pub client_id: String,
    // Will fields
    pub will_flag: bool,
    pub will_qos: QoS,
    pub will_retain: bool,
    pub will_topic: Option<String>,
    pub will_payload: Option<Bytes>,
}

#[derive(Debug, Clone)]
pub struct ConnAckPacket {
    pub session_present: bool,
    pub return_code: u8,
}

#[derive(Debug, Clone)]
pub struct PublishPacket {
    pub topic: String,
    pub packet_id: Option<u16>,
    pub payload: Bytes,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
}

#[derive(Debug, Clone)]
pub struct PubAckPacket {
    pub packet_id: u16,
}

#[derive(Debug, Clone)]
pub struct SubscribePacket {
    pub packet_id: u16,
    pub topic_filters: Vec<(String, QoS)>,
}

#[derive(Debug, Clone)]
pub struct SubAckPacket {
    pub packet_id: u16,
    pub return_codes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UnsubscribePacket {
    pub packet_id: u16,
    pub topic_filters: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UnsubAckPacket {
    pub packet_id: u16,
}

impl Packet {
    pub async fn decode<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Packet, PacketError> {
        // Read fixed header
        let first_byte = reader.read_u8().await?;
        let packet_type = PacketType::try_from((first_byte >> 4) & 0x0F)?;
        let flags = first_byte & 0x0F;

        // Read remaining length
        let remaining_length = decode_remaining_length_async(reader).await?;

        // Read the remaining packet data
        let mut packet_data = vec![0u8; remaining_length as usize];
        reader.read_exact(&mut packet_data).await?;

        let mut payload_cursor = Cursor::new(&packet_data[..]);

        let packet = match packet_type {
            PacketType::Connect => {
                let packet = decode_connect(&mut payload_cursor)?;
                Packet::Connect(packet)
            }
            PacketType::Publish => {
                let packet = decode_publish(&mut payload_cursor, flags)?;
                Packet::Publish(packet)
            }
            PacketType::PubAck => {
                if remaining_length != 2 {
                    return Err(PacketError::InvalidPacketType(packet_type as u8));
                }
                let packet_id = payload_cursor.get_u16();
                Packet::PubAck(PubAckPacket { packet_id })
            }
            PacketType::Subscribe => {
                let packet = decode_subscribe(&mut payload_cursor)?;
                Packet::Subscribe(packet)
            }
            PacketType::Unsubscribe => {
                let packet = decode_unsubscribe(&mut payload_cursor)?;
                Packet::Unsubscribe(packet)
            }
            PacketType::PingReq => Packet::PingReq,
            PacketType::Disconnect => Packet::Disconnect,
            _ => return Err(PacketError::InvalidPacketType(packet_type as u8)),
        };

        Ok(packet)
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        match self {
            Packet::ConnAck(packet) => encode_connack(packet, buf),
            Packet::Publish(packet) => encode_publish(packet, buf),
            Packet::PubAck(packet) => encode_puback(packet, buf),
            Packet::SubAck(packet) => encode_suback(packet, buf),
            Packet::UnsubAck(packet) => encode_unsuback(packet, buf),
            Packet::PingResp => encode_pingresp(buf),
            Packet::Disconnect => encode_disconnect(buf),
            _ => {} // Other packets not implemented yet
        }
    }
}

async fn decode_remaining_length_async<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<u32, PacketError> {
    let mut multiplier = 1;
    let mut value = 0;
    let mut byte_count = 0;

    loop {
        if byte_count >= 4 {
            return Err(PacketError::InvalidRemainingLength);
        }

        let byte = reader.read_u8().await?;
        value += (byte & 0x7F) as u32 * multiplier;

        if (byte & 0x80) == 0 {
            break;
        }

        multiplier *= 128;
        byte_count += 1;
    }

    Ok(value)
}

fn encode_remaining_length(length: u32, buf: &mut BytesMut) {
    let mut remaining = length;
    loop {
        let mut byte = (remaining % 128) as u8;
        remaining /= 128;
        if remaining > 0 {
            byte |= 0x80;
        }
        buf.put_u8(byte);
        if remaining == 0 {
            break;
        }
    }
}

fn decode_string(cursor: &mut Cursor<&[u8]>) -> Result<String, PacketError> {
    if cursor.remaining() < 2 {
        return Err(PacketError::IncompletePacket);
    }

    let len = cursor.get_u16() as usize;
    if cursor.remaining() < len {
        return Err(PacketError::IncompletePacket);
    }

    let mut bytes = vec![0u8; len];
    cursor.copy_to_slice(&mut bytes);

    String::from_utf8(bytes).map_err(|_| PacketError::InvalidUtf8)
}

fn encode_string(s: &str, buf: &mut BytesMut) {
    buf.put_u16(s.len() as u16);
    buf.put_slice(s.as_bytes());
}

fn decode_connect(cursor: &mut Cursor<&[u8]>) -> Result<ConnectPacket, PacketError> {
    let protocol_name = decode_string(cursor)?;
    let protocol_level = cursor.get_u8();
    let connect_flags = cursor.get_u8();
    let keep_alive = cursor.get_u16();
    let client_id = decode_string(cursor)?;

    // Parse Will fields from connect flags
    let will_flag = (connect_flags & 0x04) != 0;
    let will_qos = if will_flag {
        match (connect_flags >> 3) & 0x03 {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => return Err(PacketError::InvalidPacketFlags),
        }
    } else {
        QoS::AtMostOnce
    };
    let will_retain = (connect_flags & 0x20) != 0;

    // Read Will topic and payload if Will flag is set
    let (will_topic, will_payload) = if will_flag {
        let topic = decode_string(cursor)?;
        let payload_len = cursor.get_u16() as usize;
        let mut payload = vec![0u8; payload_len];
        cursor.copy_to_slice(&mut payload);
        (Some(topic), Some(Bytes::from(payload)))
    } else {
        (None, None)
    };

    Ok(ConnectPacket {
        protocol_name,
        protocol_level,
        clean_session: (connect_flags & 0x02) != 0,
        keep_alive,
        client_id,
        will_flag,
        will_qos,
        will_retain,
        will_topic,
        will_payload,
    })
}

fn decode_publish(cursor: &mut Cursor<&[u8]>, flags: u8) -> Result<PublishPacket, PacketError> {
    let topic = decode_string(cursor)?;
    let qos = match (flags >> 1) & 0x03 {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => return Err(PacketError::InvalidPacketType(flags)),
    };

    let packet_id = if matches!(qos, QoS::AtLeastOnce | QoS::ExactlyOnce) {
        Some(cursor.get_u16())
    } else {
        None
    };

    let payload_len = cursor.remaining();
    let mut payload = vec![0u8; payload_len];
    cursor.copy_to_slice(&mut payload);

    Ok(PublishPacket {
        topic,
        packet_id,
        payload: Bytes::from(payload),
        qos,
        retain: (flags & 0x01) != 0,
        dup: (flags & 0x08) != 0,
    })
}

fn decode_subscribe(cursor: &mut Cursor<&[u8]>) -> Result<SubscribePacket, PacketError> {
    let packet_id = cursor.get_u16();
    let mut topic_filters = Vec::new();

    while cursor.remaining() > 0 {
        let topic = decode_string(cursor)?;
        let qos_byte = cursor.get_u8();
        let qos = match qos_byte {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => return Err(PacketError::InvalidPacketType(qos_byte)),
        };
        topic_filters.push((topic, qos));
    }

    Ok(SubscribePacket {
        packet_id,
        topic_filters,
    })
}

fn decode_unsubscribe(cursor: &mut Cursor<&[u8]>) -> Result<UnsubscribePacket, PacketError> {
    let packet_id = cursor.get_u16();
    let mut topic_filters = Vec::new();

    while cursor.remaining() > 0 {
        let topic = decode_string(cursor)?;
        topic_filters.push(topic);
    }

    Ok(UnsubscribePacket {
        packet_id,
        topic_filters,
    })
}

fn encode_connack(packet: &ConnAckPacket, buf: &mut BytesMut) {
    buf.put_u8(0x20); // CONNACK packet type
    buf.put_u8(2); // Remaining length
    buf.put_u8(if packet.session_present { 0x01 } else { 0x00 });
    buf.put_u8(packet.return_code);
}

fn encode_publish(packet: &PublishPacket, buf: &mut BytesMut) {
    let mut flags = 0x30; // PUBLISH packet type
    if packet.dup {
        flags |= 0x08;
    }
    flags |= (packet.qos as u8) << 1;
    if packet.retain {
        flags |= 0x01;
    }

    buf.put_u8(flags);

    let mut remaining_length = 2 + packet.topic.len(); // Topic length
    if packet.packet_id.is_some() {
        remaining_length += 2; // Packet ID
    }
    remaining_length += packet.payload.len(); // Payload

    encode_remaining_length(remaining_length as u32, buf);
    encode_string(&packet.topic, buf);

    if let Some(packet_id) = packet.packet_id {
        buf.put_u16(packet_id);
    }

    buf.put_slice(&packet.payload);
}

fn encode_puback(packet: &PubAckPacket, buf: &mut BytesMut) {
    buf.put_u8(0x40); // PUBACK packet type
    buf.put_u8(2); // Remaining length
    buf.put_u16(packet.packet_id);
}

fn encode_suback(packet: &SubAckPacket, buf: &mut BytesMut) {
    buf.put_u8(0x90); // SUBACK packet type
    encode_remaining_length(2 + packet.return_codes.len() as u32, buf);
    buf.put_u16(packet.packet_id);
    for &code in &packet.return_codes {
        buf.put_u8(code);
    }
}

fn encode_unsuback(packet: &UnsubAckPacket, buf: &mut BytesMut) {
    buf.put_u8(0xB0); // UNSUBACK packet type
    buf.put_u8(2); // Remaining length
    buf.put_u16(packet.packet_id);
}

fn encode_disconnect(buf: &mut BytesMut) {
    buf.put_u8(0xE0); // DISCONNECT packet type
    buf.put_u8(0); // Remaining length
}

fn encode_pingresp(buf: &mut BytesMut) {
    buf.put_u8(0xD0); // PINGRESP packet type
    buf.put_u8(0); // Remaining length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_packet_encode_decode() {
        // Test basic CONNECT without Will
        let original = ConnectPacket {
            protocol_name: "MQTT".to_string(),
            protocol_level: 4,
            clean_session: true,
            keep_alive: 60,
            client_id: "test-client".to_string(),
            will_flag: false,
            will_qos: QoS::AtMostOnce,
            will_retain: false,
            will_topic: None,
            will_payload: None,
        };

        // Manually encode CONNECT packet
        let mut buf = BytesMut::new();
        buf.put_u8(0x10); // CONNECT packet type

        let mut payload = BytesMut::new();
        encode_string(&original.protocol_name, &mut payload);
        payload.put_u8(original.protocol_level);
        payload.put_u8(0x02); // Connect flags: clean_session=1
        payload.put_u16(original.keep_alive);
        encode_string(&original.client_id, &mut payload);

        encode_remaining_length(payload.len() as u32, &mut buf);
        buf.extend_from_slice(&payload);

        // Decode and verify
        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::Connect(decoded) => {
                assert_eq!(decoded.protocol_name, original.protocol_name);
                assert_eq!(decoded.protocol_level, original.protocol_level);
                assert_eq!(decoded.clean_session, original.clean_session);
                assert_eq!(decoded.keep_alive, original.keep_alive);
                assert_eq!(decoded.client_id, original.client_id);
                assert_eq!(decoded.will_flag, original.will_flag);
            }
            _ => panic!("Expected CONNECT packet"),
        }
    }

    #[tokio::test]
    async fn test_connect_packet_with_will() {
        // Test CONNECT with Will message
        let original = ConnectPacket {
            protocol_name: "MQTT".to_string(),
            protocol_level: 4,
            clean_session: false,
            keep_alive: 30,
            client_id: "will-client".to_string(),
            will_flag: true,
            will_qos: QoS::AtLeastOnce,
            will_retain: true,
            will_topic: Some("last/will".to_string()),
            will_payload: Some(Bytes::from("goodbye")),
        };

        // Manually encode CONNECT packet with Will
        let mut buf = BytesMut::new();
        buf.put_u8(0x10); // CONNECT packet type

        let mut payload = BytesMut::new();
        encode_string(&original.protocol_name, &mut payload);
        payload.put_u8(original.protocol_level);
        // Connect flags: will_retain=1, will_qos=01, will_flag=1, clean_session=0
        payload.put_u8(0x2C); // 00101100
        payload.put_u16(original.keep_alive);
        encode_string(&original.client_id, &mut payload);

        // Will topic and payload
        encode_string(original.will_topic.as_ref().unwrap(), &mut payload);
        payload.put_u16(original.will_payload.as_ref().unwrap().len() as u16);
        payload.put_slice(original.will_payload.as_ref().unwrap());

        encode_remaining_length(payload.len() as u32, &mut buf);
        buf.extend_from_slice(&payload);

        // Decode and verify
        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::Connect(decoded) => {
                assert_eq!(decoded.protocol_name, original.protocol_name);
                assert_eq!(decoded.protocol_level, original.protocol_level);
                assert_eq!(decoded.clean_session, original.clean_session);
                assert_eq!(decoded.keep_alive, original.keep_alive);
                assert_eq!(decoded.client_id, original.client_id);
                assert_eq!(decoded.will_flag, original.will_flag);
                assert!(matches!(decoded.will_qos, QoS::AtLeastOnce));
                assert_eq!(decoded.will_retain, original.will_retain);
                assert_eq!(decoded.will_topic, original.will_topic);
                assert_eq!(decoded.will_payload, original.will_payload);
            }
            _ => panic!("Expected CONNECT packet"),
        }
    }

    #[test]
    fn test_connack_packet_encode() {
        let packet = ConnAckPacket {
            session_present: true,
            return_code: 0x00, // Connection Accepted
        };

        let mut buf = BytesMut::new();
        encode_connack(&packet, &mut buf);

        assert_eq!(buf[0], 0x20); // CONNACK packet type
        assert_eq!(buf[1], 2); // Remaining length
        assert_eq!(buf[2], 0x01); // Session present flag
        assert_eq!(buf[3], 0x00); // Return code
    }

    #[tokio::test]
    async fn test_publish_packet_encode_decode_qos0() {
        let original = PublishPacket {
            topic: "test/topic".to_string(),
            packet_id: None,
            payload: Bytes::from("Hello, MQTT!"),
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
        };

        // Encode
        let mut buf = BytesMut::new();
        encode_publish(&original, &mut buf);

        // Decode
        let flags = buf[0] & 0x0F;
        let mut cursor = std::io::Cursor::new(&buf[2..]); // Skip fixed header
        let decoded = decode_publish(&mut cursor, flags).unwrap();

        assert_eq!(decoded.topic, original.topic);
        assert_eq!(decoded.packet_id, original.packet_id);
        assert_eq!(decoded.payload, original.payload);
        assert!(matches!(decoded.qos, QoS::AtMostOnce));
        assert_eq!(decoded.retain, original.retain);
        assert_eq!(decoded.dup, original.dup);
    }

    #[tokio::test]
    async fn test_publish_packet_encode_decode_qos1() {
        let original = PublishPacket {
            topic: "sensor/temp".to_string(),
            packet_id: Some(42),
            payload: Bytes::from("23.5"),
            qos: QoS::AtLeastOnce,
            retain: true,
            dup: true,
        };

        // Encode
        let mut buf = BytesMut::new();
        encode_publish(&original, &mut buf);

        // Check flags
        assert_eq!(buf[0] & 0x01, 0x01); // Retain flag
        assert_eq!((buf[0] >> 1) & 0x03, 0x01); // QoS 1
        assert_eq!(buf[0] & 0x08, 0x08); // DUP flag

        // Decode
        let flags = buf[0] & 0x0F;
        let remaining_len_bytes = if buf[1] & 0x80 != 0 { 2 } else { 1 };
        let mut cursor = std::io::Cursor::new(&buf[(1 + remaining_len_bytes)..]); // Skip fixed header
        let decoded = decode_publish(&mut cursor, flags).unwrap();

        assert_eq!(decoded.topic, original.topic);
        assert_eq!(decoded.packet_id, original.packet_id);
        assert_eq!(decoded.payload, original.payload);
        assert!(matches!(decoded.qos, QoS::AtLeastOnce));
        assert_eq!(decoded.retain, original.retain);
        assert_eq!(decoded.dup, original.dup);
    }

    #[test]
    fn test_puback_packet_encode() {
        let packet = PubAckPacket { packet_id: 12345 };

        let mut buf = BytesMut::new();
        encode_puback(&packet, &mut buf);

        assert_eq!(buf[0], 0x40); // PUBACK packet type
        assert_eq!(buf[1], 2); // Remaining length
        assert_eq!(buf[2..4], [0x30, 0x39]); // Packet ID 12345 in big-endian
    }

    #[tokio::test]
    async fn test_subscribe_packet_encode_decode() {
        let original = SubscribePacket {
            packet_id: 100,
            topic_filters: vec![
                ("topic/1".to_string(), QoS::AtMostOnce),
                ("topic/2".to_string(), QoS::AtLeastOnce),
                ("topic/+".to_string(), QoS::ExactlyOnce),
            ],
        };

        // Manually encode SUBSCRIBE packet
        let mut buf = BytesMut::new();
        buf.put_u8(0x82); // SUBSCRIBE packet type with required flags

        let mut payload = BytesMut::new();
        payload.put_u16(original.packet_id);
        for (topic, qos) in &original.topic_filters {
            encode_string(topic, &mut payload);
            payload.put_u8(*qos as u8);
        }

        encode_remaining_length(payload.len() as u32, &mut buf);
        buf.extend_from_slice(&payload);

        // Decode
        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::Subscribe(decoded) => {
                assert_eq!(decoded.packet_id, original.packet_id);
                assert_eq!(decoded.topic_filters.len(), original.topic_filters.len());
                for (i, (topic, qos)) in decoded.topic_filters.iter().enumerate() {
                    assert_eq!(topic, &original.topic_filters[i].0);
                    assert_eq!(*qos as u8, original.topic_filters[i].1 as u8);
                }
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_suback_packet_encode() {
        let packet = SubAckPacket {
            packet_id: 100,
            return_codes: vec![0x00, 0x01, 0x02, 0x80], // Various QoS levels and failure
        };

        let mut buf = BytesMut::new();
        encode_suback(&packet, &mut buf);

        assert_eq!(buf[0], 0x90); // SUBACK packet type
        assert_eq!(buf[1], 6); // Remaining length (2 + 4)
        assert_eq!(buf[2..4], [0x00, 0x64]); // Packet ID 100
        assert_eq!(&buf[4..], &[0x00, 0x01, 0x02, 0x80]); // Return codes
    }

    #[tokio::test]
    async fn test_unsubscribe_packet_encode_decode() {
        let original = UnsubscribePacket {
            packet_id: 200,
            topic_filters: vec![
                "topic/1".to_string(),
                "topic/2".to_string(),
                "topic/+".to_string(),
            ],
        };

        // Manually encode UNSUBSCRIBE packet
        let mut buf = BytesMut::new();
        buf.put_u8(0xA2); // UNSUBSCRIBE packet type with required flags

        let mut payload = BytesMut::new();
        payload.put_u16(original.packet_id);
        for topic in &original.topic_filters {
            encode_string(topic, &mut payload);
        }

        encode_remaining_length(payload.len() as u32, &mut buf);
        buf.extend_from_slice(&payload);

        // Decode
        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::Unsubscribe(decoded) => {
                assert_eq!(decoded.packet_id, original.packet_id);
                assert_eq!(decoded.topic_filters, original.topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsuback_packet_encode() {
        let packet = UnsubAckPacket { packet_id: 200 };

        let mut buf = BytesMut::new();
        encode_unsuback(&packet, &mut buf);

        assert_eq!(buf[0], 0xB0); // UNSUBACK packet type
        assert_eq!(buf[1], 2); // Remaining length
        assert_eq!(buf[2..4], [0x00, 0xC8]); // Packet ID 200
    }

    #[tokio::test]
    async fn test_pingreq_packet_decode() {
        let mut buf = BytesMut::new();
        buf.put_u8(0xC0); // PINGREQ packet type
        buf.put_u8(0); // Remaining length

        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::PingReq => {} // Success
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_pingresp_packet_encode() {
        let mut buf = BytesMut::new();
        encode_pingresp(&mut buf);

        assert_eq!(buf[0], 0xD0); // PINGRESP packet type
        assert_eq!(buf[1], 0); // Remaining length
    }

    #[tokio::test]
    async fn test_disconnect_packet_decode() {
        let mut buf = BytesMut::new();
        buf.put_u8(0xE0); // DISCONNECT packet type
        buf.put_u8(0); // Remaining length

        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let packet = Packet::decode(&mut cursor).await.unwrap();

        match packet {
            Packet::Disconnect => {} // Success
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_packet_encode() {
        let mut buf = BytesMut::new();
        encode_disconnect(&mut buf);

        assert_eq!(buf[0], 0xE0); // DISCONNECT packet type
        assert_eq!(buf[1], 0); // Remaining length
    }

    #[test]
    fn test_remaining_length_encoding() {
        // Test various lengths
        let test_cases = vec![
            (0, vec![0x00]),
            (127, vec![0x7F]),
            (128, vec![0x80, 0x01]),
            (16383, vec![0xFF, 0x7F]),
            (16384, vec![0x80, 0x80, 0x01]),
            (2097151, vec![0xFF, 0xFF, 0x7F]),
            (2097152, vec![0x80, 0x80, 0x80, 0x01]),
            (268435455, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        ];

        for (length, expected) in test_cases {
            let mut buf = BytesMut::new();
            encode_remaining_length(length, &mut buf);
            assert_eq!(buf.to_vec(), expected, "Failed for length {}", length);
        }
    }

    #[tokio::test]
    async fn test_remaining_length_decoding() {
        // Test various lengths
        let test_cases = vec![
            (vec![0x00], 0),
            (vec![0x7F], 127),
            (vec![0x80, 0x01], 128),
            (vec![0xFF, 0x7F], 16383),
            (vec![0x80, 0x80, 0x01], 16384),
            (vec![0xFF, 0xFF, 0x7F], 2097151),
            (vec![0x80, 0x80, 0x80, 0x01], 2097152),
            (vec![0xFF, 0xFF, 0xFF, 0x7F], 268435455),
        ];

        for (bytes, expected) in test_cases {
            let mut cursor = std::io::Cursor::new(bytes);
            let length = decode_remaining_length_async(&mut cursor).await.unwrap();
            assert_eq!(length, expected);
        }
    }

    #[tokio::test]
    async fn test_invalid_remaining_length() {
        // Test invalid remaining length (5 bytes)
        let bytes = vec![0xFF, 0xFF, 0xFF, 0xFF, 0x01];
        let mut cursor = std::io::Cursor::new(bytes);
        let result = decode_remaining_length_async(&mut cursor).await;
        assert!(matches!(result, Err(PacketError::InvalidRemainingLength)));
    }

    #[test]
    fn test_string_encoding_decoding() {
        let test_strings = vec![
            "",
            "MQTT",
            "Hello, World!",
            "测试中文", // Test UTF-8
            "🚀🔥💻",   // Test emojis
        ];

        for s in test_strings {
            let mut buf = BytesMut::new();
            encode_string(s, &mut buf);

            let mut cursor = Cursor::new(buf.as_ref());
            let decoded = decode_string(&mut cursor).unwrap();
            assert_eq!(decoded, s);
        }
    }

    #[test]
    fn test_qos_values() {
        // Test QoS enum values
        assert_eq!(QoS::AtMostOnce as u8, 0);
        assert_eq!(QoS::AtLeastOnce as u8, 1);
        assert_eq!(QoS::ExactlyOnce as u8, 2);
    }

    #[test]
    fn test_packet_type_conversion() {
        // Test all packet types
        let types = vec![
            (1, PacketType::Connect),
            (2, PacketType::ConnAck),
            (3, PacketType::Publish),
            (4, PacketType::PubAck),
            (5, PacketType::PubRec),
            (6, PacketType::PubRel),
            (7, PacketType::PubComp),
            (8, PacketType::Subscribe),
            (9, PacketType::SubAck),
            (10, PacketType::Unsubscribe),
            (11, PacketType::UnsubAck),
            (12, PacketType::PingReq),
            (13, PacketType::PingResp),
            (14, PacketType::Disconnect),
        ];

        for (value, expected) in types {
            let packet_type = PacketType::try_from(value).unwrap();
            assert_eq!(packet_type, expected);
            assert_eq!(packet_type as u8, value);
        }

        // Test invalid packet type
        assert!(matches!(
            PacketType::try_from(15),
            Err(PacketError::InvalidPacketType(15))
        ));
    }

    #[test]
    fn test_publish_flags_combinations() {
        // Test all combinations of PUBLISH flags
        let test_cases = vec![
            (false, QoS::AtMostOnce, false, 0x30),
            (false, QoS::AtMostOnce, true, 0x31),
            (false, QoS::AtLeastOnce, false, 0x32),
            (false, QoS::AtLeastOnce, true, 0x33),
            (false, QoS::ExactlyOnce, false, 0x34),
            (false, QoS::ExactlyOnce, true, 0x35),
            (true, QoS::AtMostOnce, false, 0x38),
            (true, QoS::AtLeastOnce, true, 0x3B),
            (true, QoS::ExactlyOnce, true, 0x3D),
        ];

        for (dup, qos, retain, expected_flags) in test_cases {
            let packet = PublishPacket {
                topic: "test".to_string(),
                packet_id: if matches!(qos, QoS::AtMostOnce) {
                    None
                } else {
                    Some(1)
                },
                payload: Bytes::from("test"),
                qos,
                retain,
                dup,
            };

            let mut buf = BytesMut::new();
            encode_publish(&packet, &mut buf);
            assert_eq!(
                buf[0], expected_flags,
                "Failed for dup={}, qos={:?}, retain={}",
                dup, qos, retain
            );
        }
    }
}
