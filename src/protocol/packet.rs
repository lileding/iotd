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

#[derive(Debug, Clone, Copy)]
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

async fn decode_remaining_length_async<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u32, PacketError> {
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

    Ok(ConnectPacket {
        protocol_name,
        protocol_level,
        clean_session: (connect_flags & 0x02) != 0,
        keep_alive,
        client_id,
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

    Ok(SubscribePacket { packet_id, topic_filters })
}

fn decode_unsubscribe(cursor: &mut Cursor<&[u8]>) -> Result<UnsubscribePacket, PacketError> {
    let packet_id = cursor.get_u16();
    let mut topic_filters = Vec::new();

    while cursor.remaining() > 0 {
        let topic = decode_string(cursor)?;
        topic_filters.push(topic);
    }

    Ok(UnsubscribePacket { packet_id, topic_filters })
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
