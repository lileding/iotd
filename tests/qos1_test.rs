use std::time::Duration;
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

async fn read_suback(stream: &mut TcpStream) {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x90); // SUBACK

    let remaining_len = header[1] as usize;
    assert!(
        remaining_len >= 3,
        "SUBACK remaining length too small: {}",
        remaining_len
    );

    let mut payload = vec![0u8; remaining_len];
    stream.read_exact(&mut payload).await.unwrap();
    // Skip packet ID (first 2 bytes) and check return code
    assert!(
        payload.len() >= 3,
        "SUBACK payload too small: {} bytes",
        payload.len()
    );
    assert!(payload[2] <= 2, "Invalid QoS granted: {}", payload[2]); // Valid QoS granted
}

#[tokio::test]
async fn test_qos1_publish_puback() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x01, // Packet ID = 1
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Should receive PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[1], 0x02); // Remaining length
    assert_eq!(puback[2], 0x00); // Packet ID MSB
    assert_eq!(puback[3], 0x01); // Packet ID LSB

    // Send DISCONNECT
    let disconnect_packet = [0xE0, 0x00];
    pub_stream.write_all(&disconnect_packet).await.unwrap();

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_subscriber_flow() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber
    let mut sub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub_stream, "subscriber").await;
    read_connack(&mut sub_stream).await;

    // Subscribe with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];
    sub_stream.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    sub_stream.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[1], 0x03); // Remaining length
    assert_eq!(suback[2], 0x00); // Packet ID MSB
    assert_eq!(suback[3], 0x01); // Packet ID LSB
    assert_eq!(suback[4], 0x01); // Granted QoS = 1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x02, // Packet ID = 2
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[3], 0x02); // Packet ID = 2

    // Subscriber should receive message with QoS=1
    let mut header = [0u8; 2];
    sub_stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let remaining_len = header[1] as usize;
    let mut payload = vec![0u8; remaining_len];
    sub_stream.read_exact(&mut payload).await.unwrap();

    // Extract packet ID from received message
    let topic_len = ((payload[0] as usize) << 8) | (payload[1] as usize);
    let packet_id_offset = 2 + topic_len;
    let received_packet_id =
        ((payload[packet_id_offset] as u16) << 8) | (payload[packet_id_offset + 1] as u16);

    // Send PUBACK from subscriber
    let subscriber_puback = [
        0x40,                              // PUBACK
        0x02,                              // Remaining length
        (received_packet_id >> 8) as u8,   // Packet ID MSB
        (received_packet_id & 0xFF) as u8, // Packet ID LSB
    ];
    sub_stream.write_all(&subscriber_puback).await.unwrap();

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_message_tracking() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect two subscribers
    let mut sub1 = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub1, "sub1").await;
    read_connack(&mut sub1).await;

    let mut sub2 = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub2, "sub2").await;
    read_connack(&mut sub2).await;

    // Both subscribe to same topic with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];

    sub1.write_all(&subscribe_packet).await.unwrap();
    sub2.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACKs
    let mut suback = [0u8; 5];
    sub1.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    sub2.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x0A, // Packet ID = 10
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[3], 0x0A); // Packet ID = 10

    // Both subscribers should receive the message with different packet IDs
    let mut received_ids = Vec::new();

    // Process sub1
    let mut header = [0u8; 2];
    sub1.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub1.read_exact(&mut payload).await.unwrap();

    // Extract packet ID for sub1
    let packet_id1 = ((payload[7] as u16) << 8) | (payload[8] as u16);
    received_ids.push(packet_id1);

    // sub1 sends PUBACK immediately
    let puback = [
        0x40,
        0x02,
        (packet_id1 >> 8) as u8,
        (packet_id1 & 0xFF) as u8,
    ];
    sub1.write_all(&puback).await.unwrap();

    // Process sub2
    let mut header = [0u8; 2];
    sub2.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub2.read_exact(&mut payload).await.unwrap();

    // Extract packet ID for sub2
    let packet_id2 = ((payload[7] as u16) << 8) | (payload[8] as u16);
    received_ids.push(packet_id2);

    // Both subscribers should get packet_id=1 (first QoS=1 message for each session)
    assert_eq!(
        received_ids[0], 1,
        "First subscriber should get packet_id=1"
    );
    assert_eq!(
        received_ids[1], 1,
        "Second subscriber should get packet_id=1"
    );

    // Now sub2 sends delayed PUBACK
    let puback = [
        0x40,
        0x02,
        (packet_id2 >> 8) as u8,
        (packet_id2 & 0xFF) as u8,
    ];
    sub2.write_all(&puback).await.unwrap();

    // Small delay to ensure PUBACKs are processed
    tokio::time::sleep(Duration::from_millis(10)).await;

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_retransmission() {
    use tracing::info;
    init_test_logging();
    // Start server with faster retransmission for testing
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    config.server.retransmission_interval_ms = 1000; // 1 second for faster testing
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber
    let mut sub = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub, "subscriber").await;
    read_connack(&mut sub).await;

    // Subscribe to topic with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];
    sub.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    sub.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x0A, // Packet ID = 10
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[3], 0x0A); // Packet ID = 10

    // Subscriber should receive the message
    let mut header = [0u8; 2];
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    // Extract packet ID
    let packet_id = ((payload[7] as u16) << 8) | (payload[8] as u16);
    assert_eq!(packet_id, 1); // Should be assigned packet_id=1

    // DO NOT send PUBACK - wait for retransmission
    info!("Waiting for retransmission...");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Should receive retransmitted message with DUP flag
    let mut header = [0u8; 2];
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x3A); // PUBLISH with QoS=1 and DUP flag (0x32 | 0x08)

    let mut payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    // Verify same packet ID
    let retrans_packet_id = ((payload[7] as u16) << 8) | (payload[8] as u16);
    assert_eq!(retrans_packet_id, packet_id);
    info!(
        "Received retransmission with packet_id={} and DUP flag",
        retrans_packet_id
    );

    // Now send PUBACK
    let puback = [0x40, 0x02, (packet_id >> 8) as u8, (packet_id & 0xFF) as u8];
    sub.write_all(&puback).await.unwrap();
    info!("Sent PUBACK for packet_id={}", packet_id);

    // Wait to ensure no more retransmissions
    info!("Waiting to verify no more retransmissions...");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Try to read - should timeout (no more retransmissions)
    let mut buffer = vec![0u8; 2];
    let result =
        tokio::time::timeout(Duration::from_millis(500), sub.read_exact(&mut buffer)).await;
    assert!(
        result.is_err(),
        "Should not receive any more messages after PUBACK"
    );

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_duplicate_handling() {
    use tracing::info;
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber
    let mut sub = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub, "subscriber").await;
    read_connack(&mut sub).await;

    // Subscribe to topic with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];
    sub.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    sub.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1 (original)
    let publish_packet = [
        0x32, // PUBLISH with QoS=1, DUP=0
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x0A, // Packet ID = 10
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[3], 0x0A); // Packet ID = 10

    // Subscriber should receive the message
    let mut header = [0u8; 2];
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    // Extract packet ID and send PUBACK
    let packet_id = ((payload[7] as u16) << 8) | (payload[8] as u16);
    let puback = [0x40, 0x02, (packet_id >> 8) as u8, (packet_id & 0xFF) as u8];
    sub.write_all(&puback).await.unwrap();

    // Now publisher sends duplicate with DUP flag set
    let duplicate_packet = [
        0x3A, // PUBLISH with QoS=1, DUP=1 (0x32 | 0x08)
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x0A, // Same Packet ID = 10
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&duplicate_packet).await.unwrap();
    info!("Sent duplicate PUBLISH with DUP flag set");

    // Publisher should still receive PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK
    assert_eq!(puback[3], 0x0A); // Packet ID = 10
    info!("Received PUBACK for duplicate");

    // Subscriber should NOT receive the duplicate message
    let mut buffer = vec![0u8; 2];
    let result =
        tokio::time::timeout(Duration::from_millis(500), sub.read_exact(&mut buffer)).await;
    assert!(
        result.is_err(),
        "Subscriber should not receive duplicate message"
    );
    info!("Confirmed: Subscriber did not receive duplicate message");

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_max_retransmission_limit() {
    use tracing::info;
    init_test_logging();
    // Start server with low retransmission limit for testing
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    config.server.retransmission_interval_ms = 500; // 500ms for faster testing
    config.server.max_retransmission_limit = 2; // Only allow 2 retries
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber
    let mut sub = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub, "subscriber").await;
    read_connack(&mut sub).await;

    // Subscribe to topic with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];
    sub.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    sub.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x0A, // Packet ID = 10
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK

    // Subscriber receives the original message
    let mut header = [0u8; 2];
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    // DO NOT send PUBACK - let it retransmit
    info!("Not sending PUBACK, waiting for retransmissions...");

    // First retransmission (retry_count = 1)
    tokio::time::sleep(Duration::from_millis(750)).await;
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x3A); // PUBLISH with QoS=1 and DUP flag
    sub.read_exact(&mut vec![0u8; header[1] as usize])
        .await
        .unwrap();
    info!("Received first retransmission");

    // Second retransmission (retry_count = 2)
    tokio::time::sleep(Duration::from_millis(500)).await;
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x3A); // PUBLISH with QoS=1 and DUP flag
    sub.read_exact(&mut vec![0u8; header[1] as usize])
        .await
        .unwrap();
    info!("Received second retransmission");

    // No more retransmissions (exceeded limit)
    let mut buffer = vec![0u8; 2];
    let result =
        tokio::time::timeout(Duration::from_millis(1000), sub.read_exact(&mut buffer)).await;
    assert!(
        result.is_err(),
        "Should not receive any more messages after exceeding retry limit"
    );
    info!("Confirmed: No more retransmissions after exceeding limit");

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos_downgrade() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect QoS=0 subscriber
    let mut sub_qos0 = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub_qos0, "subqos0").await;
    read_connack(&mut sub_qos0).await;

    // Subscribe with QoS=0
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x00, // QoS = 0
    ];
    sub_qos0.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    sub_qos0.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x00); // Granted QoS=0

    // Connect QoS=1 subscriber
    let mut sub_qos1 = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub_qos1, "subqos1").await;
    read_connack(&mut sub_qos1).await;

    // Subscribe with QoS=1
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x01, // QoS = 1
    ];
    sub_qos1.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    sub_qos1.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x01); // Granted QoS=1

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish message with QoS=1
    let publish_packet = [
        0x32, // PUBLISH with QoS=1
        0x0E, // Remaining length
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', 0x00, 0x01, // Packet ID = 1
        b'h', b'e', b'l', b'l', b'o',
    ];
    pub_stream.write_all(&publish_packet).await.unwrap();

    // Publisher receives PUBACK
    let mut puback = [0u8; 4];
    pub_stream.read_exact(&mut puback).await.unwrap();
    assert_eq!(puback[0], 0x40); // PUBACK

    // QoS=0 subscriber should receive with QoS=0 (no packet ID)
    let mut header = [0u8; 2];
    sub_qos0.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x30); // PUBLISH with QoS=0
    assert_eq!(header[1], 0x0C); // Remaining length (no packet ID)

    let mut payload = vec![0u8; header[1] as usize];
    sub_qos0.read_exact(&mut payload).await.unwrap();
    // Verify no packet ID in QoS=0 message
    assert_eq!(&payload[0..7], &[0x00, 0x05, b't', b'e', b's', b't', b'/']);
    assert_eq!(&payload[7..], b"hello");

    // QoS=1 subscriber should receive with QoS=1 (with packet ID)
    sub_qos1.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1
    assert_eq!(header[1], 0x0E); // Remaining length (with packet ID)

    let mut payload = vec![0u8; header[1] as usize];
    sub_qos1.read_exact(&mut payload).await.unwrap();

    // Extract packet ID and send PUBACK
    let packet_id = ((payload[7] as u16) << 8) | (payload[8] as u16);
    let puback = [0x40, 0x02, (packet_id >> 8) as u8, (packet_id & 0xFF) as u8];
    sub_qos1.write_all(&puback).await.unwrap();

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_qos1_message_ordering() {
    init_test_logging();

    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber
    let mut sub = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut sub, "subscriber").await;
    read_connack(&mut sub).await;

    // Subscribe with QoS=1 to a specific topic
    send_subscribe(&mut sub, "test/topic", 1, 1).await;
    read_suback(&mut sub).await;

    // Connect publisher
    let mut pub_stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut pub_stream, "publisher").await;
    read_connack(&mut pub_stream).await;

    // Publish 3 messages quickly with QoS=1 to the same topic
    for i in 1..=3 {
        let payload = format!("message{}", i);
        let mut packet = vec![
            0x32,                               // PUBLISH with QoS=1
            (2 + 10 + 2 + payload.len()) as u8, // Remaining length: topic_len(2) + topic(10) + packet_id(2) + payload
            0x00,
            0x0A, // Topic length = 10
            b't',
            b'e',
            b's',
            b't',
            b'/',
            b't',
            b'o',
            b'p',
            b'i',
            b'c',
            0x00,
            i, // Packet ID = i
        ];
        packet.extend_from_slice(payload.as_bytes());

        pub_stream.write_all(&packet).await.unwrap();

        // Read PUBACK from broker
        let mut puback = [0u8; 4];
        pub_stream.read_exact(&mut puback).await.unwrap();
        assert_eq!(puback[0], 0x40); // PUBACK
        assert_eq!(puback[2], 0x00);
        assert_eq!(puback[3], i);

        info!("Publisher sent message{} and received PUBACK", i);
    }

    // Subscriber should receive first message immediately
    let mut header = [0u8; 2];
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32); // PUBLISH with QoS=1

    let mut payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    // Verify it's message1
    let msg = String::from_utf8(payload[14..].to_vec()).unwrap(); // Skip topic + packet ID
    assert_eq!(msg, "message1");
    info!("Subscriber received message1");

    // Delay before sending PUBACK to test ordering
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send PUBACK for message1
    let puback = [0x40, 0x02, 0x00, 0x01]; // Packet ID = 1
    sub.write_all(&puback).await.unwrap();
    info!("Subscriber sent PUBACK for message1");

    // Now should receive message2
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32);

    payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    let msg = String::from_utf8(payload[14..].to_vec()).unwrap();
    assert_eq!(msg, "message2");
    info!("Subscriber received message2");

    // Send PUBACK for message2
    let puback = [0x40, 0x02, 0x00, 0x02]; // Packet ID = 2
    sub.write_all(&puback).await.unwrap();

    // Now should receive message3
    sub.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x32);

    payload = vec![0u8; header[1] as usize];
    sub.read_exact(&mut payload).await.unwrap();

    let msg = String::from_utf8(payload[14..].to_vec()).unwrap();
    assert_eq!(msg, "message3");
    info!("Subscriber received message3");

    info!("All messages received in correct order!");

    let _ = server.stop().await;
}
