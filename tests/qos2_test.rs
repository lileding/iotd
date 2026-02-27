use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::info;

static INIT: std::sync::Once = std::sync::Once::new();

pub fn init_test_logging() {
    INIT.call_once(|| {
        let log_level = std::env::var("RUST_LOG")
            .ok()
            .and_then(|s| s.parse::<tracing::Level>().ok())
            .unwrap_or(tracing::Level::INFO);

        tracing_subscriber::fmt().with_max_level(log_level).init();
    });
}

async fn send_connect(stream: &mut TcpStream, client_id: &str) {
    send_connect_with_flags(stream, client_id, 0x02).await; // Default to clean session
}

async fn send_connect_with_flags(stream: &mut TcpStream, client_id: &str, flags: u8) {
    let mut connect_packet = vec![
        0x10,                         // CONNECT packet type
        (12 + client_id.len()) as u8, // Remaining length
        0x00,
        0x04, // Protocol name length
        b'M',
        b'Q',
        b'T',
        b'T',  // Protocol name "MQTT"
        0x04,  // Protocol level (3.1.1)
        flags, // Connect flags
        0x00,
        0x3C, // Keep alive (60 seconds)
        (client_id.len() >> 8) as u8,
        client_id.len() as u8, // Client ID length
    ];
    connect_packet.extend_from_slice(client_id.as_bytes());

    stream.write_all(&connect_packet).await.unwrap();
}

async fn read_connack(stream: &mut TcpStream) {
    let mut connack = vec![0u8; 4];
    stream.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x00); // Success
}

async fn send_subscribe(stream: &mut TcpStream, topic: &str, qos: u8, packet_id: u16) {
    let mut subscribe_packet = vec![
        0x82,                    // SUBSCRIBE packet type with QoS=1
        (5 + topic.len()) as u8, // Remaining length
        (packet_id >> 8) as u8,
        (packet_id & 0xFF) as u8, // Packet ID
        (topic.len() >> 8) as u8,
        topic.len() as u8, // Topic length
    ];
    subscribe_packet.extend_from_slice(topic.as_bytes());
    subscribe_packet.push(qos); // Requested QoS

    stream.write_all(&subscribe_packet).await.unwrap();
}

async fn read_suback(stream: &mut TcpStream, expected_qos: u8) {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x90); // SUBACK

    let remaining_len = header[1] as usize;
    let mut payload = vec![0u8; remaining_len];
    stream.read_exact(&mut payload).await.unwrap();

    // Check granted QoS
    assert_eq!(
        payload[2], expected_qos,
        "Expected QoS {} but got {}",
        expected_qos, payload[2]
    );
}

/// Send QoS=2 PUBLISH and expect PUBREC
async fn send_qos2_publish(stream: &mut TcpStream, topic: &str, payload: &[u8], packet_id: u16) {
    let remaining_len = 2 + topic.len() + 2 + payload.len();
    let mut publish_packet = vec![
        0x34, // PUBLISH with QoS=2 (0011 0100)
        remaining_len as u8,
        (topic.len() >> 8) as u8,
        topic.len() as u8,
    ];
    publish_packet.extend_from_slice(topic.as_bytes());
    publish_packet.push((packet_id >> 8) as u8);
    publish_packet.push((packet_id & 0xFF) as u8);
    publish_packet.extend_from_slice(payload);

    stream.write_all(&publish_packet).await.unwrap();
}

/// Read PUBREC response and return packet_id
async fn read_pubrec(stream: &mut TcpStream) -> u16 {
    let mut pubrec = [0u8; 4];
    stream.read_exact(&mut pubrec).await.unwrap();
    assert_eq!(
        pubrec[0], 0x50,
        "Expected PUBREC (0x50), got 0x{:02x}",
        pubrec[0]
    );
    assert_eq!(pubrec[1], 2, "PUBREC remaining length should be 2");

    let packet_id = ((pubrec[2] as u16) << 8) | (pubrec[3] as u16);
    packet_id
}

/// Send PUBREL
async fn send_pubrel(stream: &mut TcpStream, packet_id: u16) {
    let pubrel = [
        0x62, // PUBREL with required flags (0110 0010)
        2,    // Remaining length
        (packet_id >> 8) as u8,
        (packet_id & 0xFF) as u8,
    ];
    stream.write_all(&pubrel).await.unwrap();
}

/// Read PUBCOMP response
async fn read_pubcomp(stream: &mut TcpStream) -> u16 {
    let mut pubcomp = [0u8; 4];
    stream.read_exact(&mut pubcomp).await.unwrap();
    assert_eq!(
        pubcomp[0], 0x70,
        "Expected PUBCOMP (0x70), got 0x{:02x}",
        pubcomp[0]
    );
    assert_eq!(pubcomp[1], 2, "PUBCOMP remaining length should be 2");

    let packet_id = ((pubcomp[2] as u16) << 8) | (pubcomp[3] as u16);
    packet_id
}

/// Read incoming PUBLISH with QoS=2
async fn read_qos2_publish(stream: &mut TcpStream) -> (u16, Vec<u8>) {
    let mut header = [0u8; 1];
    stream.read_exact(&mut header).await.unwrap();

    let qos = (header[0] >> 1) & 0x03;
    assert_eq!(qos, 2, "Expected QoS=2 PUBLISH");

    // Read remaining length
    let mut remaining_len_byte = [0u8; 1];
    stream.read_exact(&mut remaining_len_byte).await.unwrap();
    let remaining_len = remaining_len_byte[0] as usize;

    let mut payload_buf = vec![0u8; remaining_len];
    stream.read_exact(&mut payload_buf).await.unwrap();

    // Parse topic length
    let topic_len = ((payload_buf[0] as usize) << 8) | (payload_buf[1] as usize);
    let topic_end = 2 + topic_len;

    // Get packet_id (follows topic)
    let packet_id = ((payload_buf[topic_end] as u16) << 8) | (payload_buf[topic_end + 1] as u16);

    // Get message payload
    let msg_payload = payload_buf[topic_end + 2..].to_vec();

    (packet_id, msg_payload)
}

/// Send PUBREC (when receiving QoS=2 PUBLISH)
async fn send_pubrec(stream: &mut TcpStream, packet_id: u16) {
    let pubrec = [
        0x50, // PUBREC
        2,    // Remaining length
        (packet_id >> 8) as u8,
        (packet_id & 0xFF) as u8,
    ];
    stream.write_all(&pubrec).await.unwrap();
}

/// Read PUBREL (when receiving QoS=2 as subscriber)
async fn read_pubrel(stream: &mut TcpStream) -> u16 {
    let mut pubrel = [0u8; 4];
    stream.read_exact(&mut pubrel).await.unwrap();
    assert_eq!(
        pubrel[0], 0x62,
        "Expected PUBREL (0x62), got 0x{:02x}",
        pubrel[0]
    );
    assert_eq!(pubrel[1], 2, "PUBREL remaining length should be 2");

    let packet_id = ((pubrel[2] as u16) << 8) | (pubrel[3] as u16);
    packet_id
}

/// Send PUBCOMP
async fn send_pubcomp(stream: &mut TcpStream, packet_id: u16) {
    let pubcomp = [
        0x70, // PUBCOMP
        2,    // Remaining length
        (packet_id >> 8) as u8,
        (packet_id & 0xFF) as u8,
    ];
    stream.write_all(&pubcomp).await.unwrap();
}

#[tokio::test]
async fn test_qos2_publish_full_handshake() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();
    info!("Test server started on {}", address);

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "qos2publisher").await;
    read_connack(&mut pub_stream).await;

    // Send QoS=2 PUBLISH
    let packet_id = 1;
    send_qos2_publish(&mut pub_stream, "test/qos2", b"hello qos2", packet_id).await;

    // Expect PUBREC
    let recv_packet_id = read_pubrec(&mut pub_stream).await;
    assert_eq!(recv_packet_id, packet_id, "PUBREC packet_id mismatch");

    // Send PUBREL
    send_pubrel(&mut pub_stream, packet_id).await;

    // Expect PUBCOMP
    let comp_packet_id = read_pubcomp(&mut pub_stream).await;
    assert_eq!(comp_packet_id, packet_id, "PUBCOMP packet_id mismatch");

    info!("QoS=2 full handshake completed successfully");
    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos2_subscriber_receives_message() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();
    info!("Test server started on {}", address);

    // Connect subscriber
    let mut sub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub_stream, "qos2subscriber").await;
    read_connack(&mut sub_stream).await;

    // Subscribe with QoS=2
    send_subscribe(&mut sub_stream, "test/qos2", 2, 1).await;
    read_suback(&mut sub_stream, 2).await;

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "qos2publisher").await;
    read_connack(&mut pub_stream).await;

    // Publisher: QoS=2 PUBLISH handshake
    let packet_id = 1;
    send_qos2_publish(&mut pub_stream, "test/qos2", b"qos2 message", packet_id).await;
    let _ = read_pubrec(&mut pub_stream).await;
    send_pubrel(&mut pub_stream, packet_id).await;
    let _ = read_pubcomp(&mut pub_stream).await;

    // Subscriber: receive QoS=2 PUBLISH and complete handshake
    let (sub_packet_id, payload) = read_qos2_publish(&mut sub_stream).await;
    assert_eq!(payload, b"qos2 message");

    // Subscriber: send PUBREC
    send_pubrec(&mut sub_stream, sub_packet_id).await;

    // Subscriber: receive PUBREL
    let rel_packet_id = read_pubrel(&mut sub_stream).await;
    assert_eq!(rel_packet_id, sub_packet_id);

    // Subscriber: send PUBCOMP
    send_pubcomp(&mut sub_stream, sub_packet_id).await;

    info!("QoS=2 subscriber received message successfully");
    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos2_grants_qos2_subscription() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect
    let mut stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut stream, "qos2test").await;
    read_connack(&mut stream).await;

    // Subscribe with QoS=2 and verify it's granted
    send_subscribe(&mut stream, "test/topic", 2, 1).await;
    read_suback(&mut stream, 2).await;

    info!("QoS=2 subscription granted successfully");
    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos2_downgrade_to_qos1() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber with QoS=1
    let mut sub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub_stream, "qos1subscriber").await;
    read_connack(&mut sub_stream).await;

    // Subscribe with QoS=1
    send_subscribe(&mut sub_stream, "test/qos", 1, 1).await;
    read_suback(&mut sub_stream, 1).await;

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "qos2publisher").await;
    read_connack(&mut pub_stream).await;

    // Publisher: send QoS=2 PUBLISH
    let packet_id = 1;
    send_qos2_publish(&mut pub_stream, "test/qos", b"downgrade test", packet_id).await;
    let _ = read_pubrec(&mut pub_stream).await;
    send_pubrel(&mut pub_stream, packet_id).await;
    let _ = read_pubcomp(&mut pub_stream).await;

    // Subscriber: should receive QoS=1 (downgraded)
    let mut header = [0u8; 1];
    sub_stream.read_exact(&mut header).await.unwrap();

    let qos = (header[0] >> 1) & 0x03;
    assert_eq!(qos, 1, "Expected QoS to be downgraded to 1, got {}", qos);

    info!("QoS=2 correctly downgraded to QoS=1 for subscriber");
    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos2_duplicate_publish_handling() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "qos2duppub").await;
    read_connack(&mut pub_stream).await;

    // Send QoS=2 PUBLISH
    let packet_id = 1;
    send_qos2_publish(&mut pub_stream, "test/qos2", b"original", packet_id).await;

    // Get PUBREC
    let _ = read_pubrec(&mut pub_stream).await;

    // Send duplicate PUBLISH with same packet_id and DUP flag
    let topic = b"test/qos2"; // 9 bytes
    let payload = b"duplicate"; // 9 bytes
    let remaining_len = 2 + topic.len() + 2 + payload.len(); // topic_len(2) + topic + packet_id(2) + payload
    let mut dup_publish = vec![
        0x3C, // PUBLISH with QoS=2 and DUP=1 (0011 1100)
        remaining_len as u8,
        (topic.len() >> 8) as u8,
        topic.len() as u8, // topic length
    ];
    dup_publish.extend_from_slice(topic);
    dup_publish.push((packet_id >> 8) as u8);
    dup_publish.push((packet_id & 0xFF) as u8);
    dup_publish.extend_from_slice(payload);
    pub_stream.write_all(&dup_publish).await.unwrap();

    // Should get another PUBREC for the duplicate
    let dup_packet_id = read_pubrec(&mut pub_stream).await;
    assert_eq!(
        dup_packet_id, packet_id,
        "PUBREC for duplicate should have same packet_id"
    );

    // Complete handshake
    send_pubrel(&mut pub_stream, packet_id).await;
    let _ = read_pubcomp(&mut pub_stream).await;

    info!("QoS=2 duplicate PUBLISH handled correctly");
    let _ = server.stop().await;
}
