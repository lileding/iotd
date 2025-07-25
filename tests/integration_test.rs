use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::Level;

static INIT: std::sync::Once = std::sync::Once::new();

pub fn init_test_logging() {
    INIT.call_once(|| {
        let log_level = std::env::var("RUST_LOG")
            .ok()
            .and_then(|s| s.parse::<Level>().ok())
            .unwrap_or(Level::INFO);

        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .init();
    });
}

#[tokio::test]
async fn test_basic_connect_and_disconnect() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect client
    let mut stream = TcpStream::connect(&address).await.unwrap();

    // Send CONNECT packet (MQTT v3.1.1)
    let connect_packet = [
        0x10, // CONNECT packet type
        0x10, // Remaining length = 16
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x3C, // Keep alive = 60 seconds
        0x00, 0x04, // Client ID length
        b't', b'e', b's', b't', // Client ID "test"
    ];

    stream.write_all(&connect_packet).await.unwrap();
    stream.flush().await.unwrap();

    // Read CONNACK
    let mut response = [0u8; 4];
    stream.read_exact(&mut response).await.unwrap();

    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[1], 0x02); // Remaining length
    assert_eq!(response[2], 0x00); // Session present = false
    assert_eq!(response[3], 0x00); // Return code = accepted

    // Send DISCONNECT
    let disconnect_packet = [0xE0, 0x00]; // DISCONNECT with 0 remaining length
    stream.write_all(&disconnect_packet).await.unwrap();
    stream.flush().await.unwrap();

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_publish_subscribe() {
    init_test_logging();
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect subscriber first
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Connect publisher
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    // Subscribe to topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut response = [0u8; 5];
    match timeout(Duration::from_secs(1), subscriber.read_exact(&mut response)).await {
        Ok(Ok(_)) => {
            assert_eq!(response[0], 0x90); // SUBACK
            assert_eq!(response[2], 0x00); // Packet ID = 1 (high byte) 
            assert_eq!(response[3], 0x01); // Packet ID = 1 (low byte)
            assert_eq!(response[4], 0x00); // Return code = Maximum QoS 0
        }
        Ok(Err(e)) => panic!("Failed to read SUBACK: {}", e),
        Err(_) => panic!("Timeout reading SUBACK"),
    }

    // Publish message
    let mut publish_packet = vec![
        0x30, // PUBLISH packet type (QoS 0)
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/"); // Topic "test/"
    publish_packet.extend_from_slice(b"hello"); // Payload "hello"

    publisher.write_all(&publish_packet).await.unwrap();

    // Read published message on subscriber
    let mut header = [0u8; 2];
    timeout(Duration::from_secs(1), subscriber.read_exact(&mut header))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(header[0], 0x30); // PUBLISH

    let remaining_length = header[1] as usize;
    let mut payload = vec![0u8; remaining_length];
    timeout(Duration::from_secs(1), subscriber.read_exact(&mut payload))
        .await
        .unwrap()
        .unwrap();

    // Verify topic and message
    assert_eq!(payload[2..7], *b"test/");
    assert_eq!(payload[7..], *b"hello");

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_clean_session_false_persistence() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // First connection with clean_session=false
    let mut stream1 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream1, "persist", 0x00).await; // clean_session=false
    let session_present = read_connack_with_session_present(&mut stream1).await;
    assert_eq!(session_present, false); // First connection, no session

    // Subscribe to a topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    stream1.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut response = [0u8; 5];
    stream1.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x90); // SUBACK

    // Disconnect
    drop(stream1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with same client ID and clean_session=false
    let mut stream2 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream2, "persist", 0x00).await; // clean_session=false
    let session_present = read_connack_with_session_present(&mut stream2).await;
    assert_eq!(session_present, true); // Session should be present

    // Publish to the topic (should be received since subscription persisted)
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    let mut publish_packet = vec![
        0x30, // PUBLISH packet type (QoS 0)
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/"); // Topic "test/"
    publish_packet.extend_from_slice(b"hello"); // Payload "hello"
    publisher.write_all(&publish_packet).await.unwrap();

    // Should receive the message on stream2 (persisted subscription)
    let mut header = [0u8; 2];
    match timeout(Duration::from_secs(1), stream2.read_exact(&mut header)).await {
        Ok(Ok(_)) => {
            assert_eq!(header[0], 0x30); // PUBLISH
            let remaining_length = header[1] as usize;
            let mut payload = vec![0u8; remaining_length];
            stream2.read_exact(&mut payload).await.unwrap();
            assert_eq!(payload[7..], *b"hello"); // Message received
        }
        _ => panic!("Did not receive message on persisted subscription"),
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_clean_session_true_no_persistence() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // First connection with clean_session=true
    let mut stream1 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream1, "clean", 0x02).await; // clean_session=true
    let session_present = read_connack_with_session_present(&mut stream1).await;
    assert_eq!(session_present, false); // Always false for clean_session=true

    // Subscribe to a topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    stream1.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut response = [0u8; 5];
    stream1.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x90); // SUBACK

    // Disconnect
    drop(stream1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with same client ID and clean_session=true
    let mut stream2 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream2, "clean", 0x02).await; // clean_session=true
    let session_present = read_connack_with_session_present(&mut stream2).await;
    assert_eq!(session_present, false); // Always false for clean_session=true

    // Publish to the topic (should NOT be received since subscription was cleared)
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    let mut publish_packet = vec![
        0x30, // PUBLISH packet type (QoS 0)
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/"); // Topic "test/"
    publish_packet.extend_from_slice(b"hello"); // Payload "hello"
    publisher.write_all(&publish_packet).await.unwrap();

    // Should NOT receive the message on stream2 (no subscription)
    let mut header = [0u8; 2];
    match timeout(Duration::from_millis(500), stream2.read_exact(&mut header)).await {
        Ok(_) => panic!("Should not receive message after clean session"),
        Err(_) => {} // Expected timeout
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_session_takeover() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // First client connects with clean_session=false to test subscription persistence
    let mut stream1 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream1, "takeover", 0x00).await; // clean_session=false
    read_connack(&mut stream1).await;

    // Subscribe to a topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    stream1.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut response = [0u8; 5];
    stream1.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x90); // SUBACK

    // Small delay to ensure first client is fully established
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second client connects with same client ID (takeover) - also with clean_session=false
    let mut stream2 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream2, "takeover", 0x00).await; // clean_session=false
    let session_present = read_connack_with_session_present(&mut stream2).await;
    assert_eq!(session_present, true); // Should have existing session

    // First client should receive DISCONNECT
    let mut disconnect = [0u8; 2];
    match timeout(Duration::from_secs(1), stream1.read_exact(&mut disconnect)).await {
        Ok(Ok(_)) => {
            assert_eq!(disconnect[0], 0xE0); // DISCONNECT packet
            assert_eq!(disconnect[1], 0x00); // No payload
        }
        _ => panic!("First client should receive DISCONNECT on takeover"),
    }

    // Verify second client took over the session - publish message
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    let mut publish_packet = vec![
        0x30, // PUBLISH packet type (QoS 0)
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/");
    publish_packet.extend_from_slice(b"hello");
    publisher.write_all(&publish_packet).await.unwrap();

    // Second client should receive the message (took over subscription)
    let mut header = [0u8; 2];
    match timeout(Duration::from_secs(1), stream2.read_exact(&mut header)).await {
        Ok(Ok(_)) => {
            assert_eq!(header[0], 0x30); // PUBLISH
            let remaining_length = header[1] as usize;
            let mut payload = vec![0u8; remaining_length];
            stream2.read_exact(&mut payload).await.unwrap();
            assert_eq!(payload[7..], *b"hello");
        }
        _ => panic!("Second client should receive message after takeover"),
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_clean_session_transition() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // First connection with clean_session=false
    let mut stream1 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream1, "trans", 0x00).await; // clean_session=false
    let session_present = read_connack_with_session_present(&mut stream1).await;
    assert_eq!(session_present, false);

    // Subscribe to a topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    stream1.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut response = [0u8; 5];
    stream1.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x90); // SUBACK

    // Disconnect
    drop(stream1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with clean_session=true (should clear persisted state)
    let mut stream2 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream2, "trans", 0x02).await; // clean_session=true
    let session_present = read_connack_with_session_present(&mut stream2).await;
    assert_eq!(session_present, false); // Always false for clean_session=true

    // Disconnect again
    drop(stream2);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with clean_session=false again (should have no persisted state)
    let mut stream3 = TcpStream::connect(&address).await.unwrap();
    send_connect_with_flags(&mut stream3, "trans", 0x00).await; // clean_session=false
    let session_present = read_connack_with_session_present(&mut stream3).await;
    assert_eq!(session_present, false); // No session since it was cleared

    // Publish to verify no subscription exists
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    let mut publish_packet = vec![
        0x30, // PUBLISH packet type (QoS 0)
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/");
    publish_packet.extend_from_slice(b"hello");
    publisher.write_all(&publish_packet).await.unwrap();

    // Should NOT receive message (no subscription after clean session)
    let mut header = [0u8; 2];
    match timeout(Duration::from_millis(500), stream3.read_exact(&mut header)).await {
        Ok(_) => panic!("Should not receive message after clean session cleared subscriptions"),
        Err(_) => {} // Expected timeout
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_keep_alive_timeout() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect with 1 second keep-alive
    let mut stream = TcpStream::connect(&address).await.unwrap();
    let connect_packet = vec![
        0x10, // CONNECT packet type
        16, // Remaining length
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x01, // Keep alive = 1 second
        0x00, 4, // Client ID length
        b't', b'e', b's', b't', // Client ID
    ];
    stream.write_all(&connect_packet).await.unwrap();

    // Read CONNACK
    let mut response = [0u8; 4];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x20); // CONNACK
    assert_eq!(response[3], 0x00); // Accepted

    // Wait for 1.6 seconds (more than 1.5x keep-alive)
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // Try to read - should get disconnected
    let mut buf = [0u8; 1];
    match timeout(Duration::from_millis(500), stream.read_exact(&mut buf)).await {
        Ok(Ok(0)) | Err(_) => {}, // Connection closed or timeout - both are acceptable
        Ok(Ok(_)) => panic!("Should have been disconnected due to keep-alive timeout"),
        Ok(Err(_)) => {}, // IO error also acceptable (connection closed)
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_keep_alive_with_pingreq() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect with 1 second keep-alive
    let mut stream = TcpStream::connect(&address).await.unwrap();
    let connect_packet = vec![
        0x10, // CONNECT packet type
        16, // Remaining length
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x01, // Keep alive = 1 second
        0x00, 4, // Client ID length
        b't', b'e', b's', b't', // Client ID
    ];
    stream.write_all(&connect_packet).await.unwrap();

    // Read CONNACK
    let mut response = [0u8; 4];
    stream.read_exact(&mut response).await.unwrap();

    // Send PINGREQ every 800ms (within keep-alive)
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(800)).await;

        let pingreq = [0xC0, 0x00]; // PINGREQ
        stream.write_all(&pingreq).await.unwrap();

        // Read PINGRESP
        let mut pingresp = [0u8; 2];
        match timeout(Duration::from_millis(500), stream.read_exact(&mut pingresp)).await {
            Ok(Ok(_)) => {
                assert_eq!(pingresp[0], 0xD0); // PINGRESP
                assert_eq!(pingresp[1], 0x00);
            }
            _ => panic!("Should receive PINGRESP"),
        }
    }

    // Connection should still be alive
    let disconnect = [0xE0, 0x00]; // DISCONNECT
    stream.write_all(&disconnect).await.unwrap();

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_keep_alive_zero_disabled() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect with keep-alive = 0 (disabled)
    let mut stream = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut stream, "test").await;
    read_connack(&mut stream).await;

    // Wait for 5 seconds - should not disconnect
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Send PINGREQ to check connection is still alive
    let pingreq = [0xC0, 0x00];
    stream.write_all(&pingreq).await.unwrap();

    // Should receive PINGRESP
    let mut pingresp = [0u8; 2];
    match timeout(Duration::from_millis(500), stream.read_exact(&mut pingresp)).await {
        Ok(Ok(_)) => {
            assert_eq!(pingresp[0], 0xD0); // PINGRESP
        }
        _ => panic!("Connection should still be alive with keep-alive=0"),
    }

    let _ = server.stop().await;
}


async fn send_connect(stream: &mut TcpStream, client_id: &str) {
    send_connect_with_flags(stream, client_id, 0x02).await; // Default to clean session
}

async fn send_connect_with_flags(stream: &mut TcpStream, client_id: &str, flags: u8) {
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        (12 + client_id.len()) as u8, // Remaining length
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        flags, // Connect flags
        0x00, 0x3C, // Keep alive = 60 seconds
        0x00, client_id.len() as u8, // Client ID length
    ];
    connect_packet.extend_from_slice(client_id.as_bytes());

    stream.write_all(&connect_packet).await.unwrap();
    stream.flush().await.unwrap();
}

async fn read_connack(stream: &mut TcpStream) {
    let mut response = [0u8; 4];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[3], 0x00); // Return code = accepted
}

async fn read_connack_with_session_present(stream: &mut TcpStream) -> bool {
    let mut response = [0u8; 4];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[3], 0x00); // Return code = accepted
    response[2] & 0x01 == 0x01 // Session present flag
}

#[tokio::test]
async fn test_retained_message_basic() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Publisher sends retained message
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    // Publish with retain=true
    let mut publish_packet = vec![
        0x31, // PUBLISH packet type with retain flag
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish_packet.extend_from_slice(b"test/"); // Topic "test/"
    publish_packet.extend_from_slice(b"hello"); // Payload "hello"
    publisher.write_all(&publish_packet).await.unwrap();

    // Small delay to ensure message is stored
    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber connects
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe to topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type with flags
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter "test/"
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK (should always come first now)
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[1], 0x03); // Remaining length
    assert_eq!(suback[2], 0x00); // Packet ID MSB
    assert_eq!(suback[3], 0x01); // Packet ID LSB
    assert_eq!(suback[4], 0x00); // Return code (success)

    // Now read retained message
    let mut header = [0u8; 2];
    subscriber.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x31); // PUBLISH with retain flag
    let mut payload = vec![0u8; header[1] as usize];
    subscriber.read_exact(&mut payload).await.unwrap();
    assert_eq!(payload[7..], *b"hello"); // Retained message received

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_retained_message_update() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Publisher sends first retained message
    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    // Publish first retained message
    let mut publish1 = vec![
        0x31, // PUBLISH with retain
        0x0E, // Remaining length = 14
        0x00, 0x05, // Topic length = 5
    ];
    publish1.extend_from_slice(b"test/");
    publish1.extend_from_slice(b"first!!");
    publisher.write_all(&publish1).await.unwrap();
    publisher.flush().await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Update retained message
    let mut publish2 = vec![
        0x31, // PUBLISH with retain
        0x0E, // Remaining length = 14 (2 + 5 + 7)
        0x00, 0x05, // Topic length = 5
    ];
    publish2.extend_from_slice(b"test/");
    publish2.extend_from_slice(b"updated");
    publisher.write_all(&publish2).await.unwrap();
    publisher.flush().await.unwrap(); // Ensure data is sent

    tokio::time::sleep(Duration::from_millis(100)).await; // Give more time

    // New subscriber should get updated message
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe
    let subscribe_packet = [
        0x82, // SUBSCRIBE
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK (should always come first now)
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[1], 0x03); // Remaining length
    assert_eq!(suback[2], 0x00); // Packet ID MSB
    assert_eq!(suback[3], 0x01); // Packet ID LSB
    assert_eq!(suback[4], 0x00); // Return code (success)

    // Now read retained message
    let mut header = [0u8; 2];
    subscriber.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x31); // PUBLISH with retain
    let mut payload = vec![0u8; header[1] as usize];
    subscriber.read_exact(&mut payload).await.unwrap();
    assert_eq!(payload[7..], *b"updated"); // Updated message

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_retained_message_delete() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    // Publish retained message
    let mut publish = vec![
        0x31, // PUBLISH with retain
        0x0C, // Remaining length = 12
        0x00, 0x05, // Topic length = 5
    ];
    publish.extend_from_slice(b"test/");
    publish.extend_from_slice(b"hello");
    publisher.write_all(&publish).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Delete retained message (empty payload with retain=true)
    let delete = vec![
        0x31, // PUBLISH with retain
        0x07, // Remaining length = 7 (no payload)
        0x00, 0x05, // Topic length = 5
        b't', b'e', b's', b't', b'/', // Topic
                                      // No payload
    ];
    publisher.write_all(&delete).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // New subscriber should NOT receive any retained message
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe
    let subscribe_packet = [
        0x82, // SUBSCRIBE
        0x0A, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x05, // Topic filter length = 5
        b't', b'e', b's', b't', b'/', // Topic filter
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();

    // Should NOT receive any retained message
    let mut header = [0u8; 2];
    match timeout(Duration::from_millis(500), subscriber.read_exact(&mut header)).await {
        Ok(_) => panic!("Should not receive retained message after deletion"),
        Err(_) => {} // Expected timeout
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_retained_message_wildcard() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    let mut publisher = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut publisher, "pub").await;
    read_connack(&mut publisher).await;

    // Publish multiple retained messages
    let topics = [
        ("home/room1/temp", "20C"),
        ("home/room2/temp", "22C"),
        ("home/room1/humidity", "60%"),
    ];

    for (topic, payload) in &topics {
        let mut publish = vec![
            0x31, // PUBLISH with retain
            (2 + topic.len() + payload.len()) as u8, // Remaining length
            (topic.len() >> 8) as u8, // Topic length high byte
            (topic.len() & 0xFF) as u8, // Topic length low byte
        ];
        publish.extend_from_slice(topic.as_bytes());
        publish.extend_from_slice(payload.as_bytes());
        publisher.write_all(&publish).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Subscribe with wildcard
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe to "home/+/temp"
    let subscribe_packet = vec![
        0x82, // SUBSCRIBE
        0x10, // Remaining length = 16
        0x00, 0x01, // Packet ID = 1
        0x00, 0x0B, // Topic filter length = 11
        b'h', b'o', b'm', b'e', b'/', b'+', b'/', b't', b'e', b'm', b'p', // "home/+/temp" (11 chars)
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK first
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[1], 0x03); // Remaining length
    assert_eq!(suback[2], 0x00); // Packet ID MSB
    assert_eq!(suback[3], 0x01); // Packet ID LSB
    assert_eq!(suback[4], 0x00); // Return code (success)

    // Now read 2 retained messages
    let mut retained_count = 0;
    for _ in 0..2 {
        let mut header = [0u8; 2];
        match timeout(Duration::from_secs(1), subscriber.read_exact(&mut header)).await {
            Ok(Ok(_)) => {
                assert_eq!(header[0], 0x31); // PUBLISH with retain
                let mut payload = vec![0u8; header[1] as usize];
                subscriber.read_exact(&mut payload).await.unwrap();
                retained_count += 1;
            }
            _ => break,
        }
    }

    assert_eq!(retained_count, 2, "Should receive 2 retained messages for wildcard subscription");

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_will_message_on_disconnect() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Publisher with Will message
    let mut publisher = TcpStream::connect(&address).await.unwrap();

    // Send CONNECT with Will
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length placeholder - will update
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level (MQTT 3.1.1)
    connect_packet.push(0x0E); // Connect flags: Will flag=1, Will QoS=0, Will retain=1, Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x0A]); // Keep-alive = 10 seconds

    // Payload
    connect_packet.extend_from_slice(&[0x00, 0x03]); // Client ID length
    connect_packet.extend_from_slice(b"pub"); // Client ID
    connect_packet.extend_from_slice(&[0x00, 0x0A]); // Will topic length
    connect_packet.extend_from_slice(b"will/topic"); // Will topic
    connect_packet.extend_from_slice(&[0x00, 0x07]); // Will message length
    connect_packet.extend_from_slice(b"offline"); // Will message

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    publisher.write_all(&connect_packet).await.unwrap();
    read_connack(&mut publisher).await;

    // Subscriber to receive Will message
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe to Will topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE packet type
        0x0F, // Remaining length
        0x00, 0x01, // Packet ID = 1
        0x00, 0x0A, // Topic filter length
        b'w', b'i', b'l', b'l', b'/', b't', b'o', b'p', b'i', b'c', // "will/topic"
        0x00, // QoS = 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK

    // Abruptly close publisher connection (no DISCONNECT)
    drop(publisher);

    // Small delay to allow Will message to be published
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should receive Will message
    let mut header = [0u8; 2];
    subscriber.read_exact(&mut header).await.unwrap();
    // Will message with retain=true should be delivered as retain=false to current subscribers
    assert_eq!(header[0], 0x30); // PUBLISH without retain flag

    let mut payload = vec![0u8; header[1] as usize];
    subscriber.read_exact(&mut payload).await.unwrap();

    // Verify it's the Will message
    let topic_len = ((payload[0] as usize) << 8) | (payload[1] as usize);
    let topic = std::str::from_utf8(&payload[2..2+topic_len]).unwrap();
    assert_eq!(topic, "will/topic");

    let message = &payload[2+topic_len..];
    assert_eq!(message, b"offline");

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_will_message_not_sent_on_disconnect() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Publisher with Will message
    let mut publisher = TcpStream::connect(&address).await.unwrap();

    // Send CONNECT with Will
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length placeholder
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x06); // Connect flags: Will flag=1, Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x0A]); // Keep-alive

    // Payload
    connect_packet.extend_from_slice(&[0x00, 0x03]); // Client ID length
    connect_packet.extend_from_slice(b"pub"); // Client ID
    connect_packet.extend_from_slice(&[0x00, 0x0A]); // Will topic length
    connect_packet.extend_from_slice(b"will/topic"); // Will topic
    connect_packet.extend_from_slice(&[0x00, 0x07]); // Will message length
    connect_packet.extend_from_slice(b"offline"); // Will message

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    publisher.write_all(&connect_packet).await.unwrap();
    read_connack(&mut publisher).await;

    // Subscriber
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe to Will topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE
        0x0F, // Remaining length
        0x00, 0x01, // Packet ID
        0x00, 0x0A, // Topic filter length
        b'w', b'i', b'l', b'l', b'/', b't', b'o', b'p', b'i', b'c',
        0x00, // QoS
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();

    // Send DISCONNECT from publisher
    let disconnect = [0xE0, 0x00]; // DISCONNECT packet
    publisher.write_all(&disconnect).await.unwrap();

    // Small delay
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should NOT receive Will message
    let mut header = [0u8; 2];
    match timeout(Duration::from_millis(500), subscriber.read_exact(&mut header)).await {
        Ok(_) => panic!("Should not receive Will message after normal DISCONNECT"),
        Err(_) => {} // Expected timeout
    }

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_will_message_on_keepalive_timeout() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Publisher with Will message and short keep-alive
    let mut publisher = TcpStream::connect(&address).await.unwrap();

    // Send CONNECT with Will and 2-second keep-alive
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length placeholder
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x06); // Connect flags: Will flag=1, Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x02]); // Keep-alive = 2 seconds

    // Payload
    connect_packet.extend_from_slice(&[0x00, 0x03]); // Client ID length
    connect_packet.extend_from_slice(b"pub"); // Client ID
    connect_packet.extend_from_slice(&[0x00, 0x0C]); // Will topic length
    connect_packet.extend_from_slice(b"will/timeout"); // Will topic
    connect_packet.extend_from_slice(&[0x00, 0x07]); // Will message length
    connect_packet.extend_from_slice(b"timeout"); // Will message

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    publisher.write_all(&connect_packet).await.unwrap();
    read_connack(&mut publisher).await;

    // Subscriber
    let mut subscriber = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut subscriber, "sub").await;
    read_connack(&mut subscriber).await;

    // Subscribe to Will topic
    let subscribe_packet = [
        0x82, // SUBSCRIBE
        0x11, // Remaining length
        0x00, 0x01, // Packet ID
        0x00, 0x0C, // Topic filter length
        b'w', b'i', b'l', b'l', b'/', b't', b'i', b'm', b'e', b'o', b'u', b't',
        0x00, // QoS
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();

    // Wait for keep-alive timeout (2s * 1.5 = 3s)
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Subscriber should receive Will message
    let mut header = [0u8; 2];
    subscriber.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x30); // PUBLISH without retain

    let mut payload = vec![0u8; header[1] as usize];
    subscriber.read_exact(&mut payload).await.unwrap();

    // Verify it's the Will message
    let topic_len = ((payload[0] as usize) << 8) | (payload[1] as usize);
    let topic = std::str::from_utf8(&payload[2..2+topic_len]).unwrap();
    assert_eq!(topic, "will/timeout");

    let message = &payload[2+topic_len..];
    assert_eq!(message, b"timeout");

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_protocol_validation() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Test 1: Invalid protocol name
    let mut client = TcpStream::connect(&address).await.unwrap();

    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length (will update)
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"HTTP"); // Wrong protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x02); // Connect flags: Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x3C]); // Keep-alive = 60

    // Payload
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Client ID length
    connect_packet.extend_from_slice(b"test"); // Client ID

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    client.write_all(&connect_packet).await.unwrap();

    // Should receive CONNACK with error code 0x01
    let mut connack = vec![0u8; 4];
    client.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x01); // UNACCEPTABLE_PROTOCOL_VERSION

    // Connection should be closed
    let mut buf = [0u8; 1];
    assert!(client.read(&mut buf).await.unwrap() == 0);

    // Test 2: Invalid protocol level
    let mut client = TcpStream::connect(&address).await.unwrap();

    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length (will update)
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Correct protocol name
    connect_packet.push(0x03); // Wrong protocol level (3 instead of 4)
    connect_packet.push(0x02); // Connect flags: Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x3C]); // Keep-alive = 60

    // Payload
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Client ID length
    connect_packet.extend_from_slice(b"test"); // Client ID

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    client.write_all(&connect_packet).await.unwrap();

    // Should receive CONNACK with error code 0x01
    let mut connack = vec![0u8; 4];
    client.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x01); // UNACCEPTABLE_PROTOCOL_VERSION

    // Connection should be closed
    let mut buf = [0u8; 1];
    assert!(client.read(&mut buf).await.unwrap() == 0);

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_client_id_validation() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Test 1: Client ID too long (more than 23 bytes)
    let mut client = TcpStream::connect(&address).await.unwrap();

    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length (will update)
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x02); // Connect flags: Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x3C]); // Keep-alive = 60

    // Payload with long client ID
    let long_client_id = "a".repeat(24); // 24 bytes, exceeds limit
    connect_packet.extend_from_slice(&[0x00, 0x18]); // Client ID length = 24
    connect_packet.extend_from_slice(long_client_id.as_bytes());

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    client.write_all(&connect_packet).await.unwrap();

    // Should receive CONNACK with error code 0x02
    let mut connack = vec![0u8; 4];
    client.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x02); // IDENTIFIER_REJECTED

    // Connection should be closed
    let mut buf = [0u8; 1];
    assert!(client.read(&mut buf).await.unwrap() == 0);

    // Test 2: Client ID with invalid characters
    let mut client = TcpStream::connect(&address).await.unwrap();

    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length (will update)
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x02); // Connect flags: Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x3C]); // Keep-alive = 60

    // Payload with invalid client ID
    connect_packet.extend_from_slice(&[0x00, 0x07]); // Client ID length
    connect_packet.extend_from_slice(b"test-id"); // Contains hyphen (invalid)

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    client.write_all(&connect_packet).await.unwrap();

    // Should receive CONNACK with error code 0x02
    let mut connack = vec![0u8; 4];
    client.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x02); // IDENTIFIER_REJECTED

    // Connection should be closed
    let mut buf = [0u8; 1];
    assert!(client.read(&mut buf).await.unwrap() == 0);

    // Test 3: Valid client ID should work
    let mut client = TcpStream::connect(&address).await.unwrap();

    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        0x00, // Remaining length (will update)
    ];

    // Variable header
    connect_packet.extend_from_slice(&[0x00, 0x04]); // Protocol name length
    connect_packet.extend_from_slice(b"MQTT"); // Protocol name
    connect_packet.push(0x04); // Protocol level
    connect_packet.push(0x02); // Connect flags: Clean session=1
    connect_packet.extend_from_slice(&[0x00, 0x3C]); // Keep-alive = 60

    // Payload with valid client ID
    connect_packet.extend_from_slice(&[0x00, 0x09]); // Client ID length
    connect_packet.extend_from_slice(b"Client123"); // Valid: alphanumeric only

    // Update remaining length
    let remaining_len = connect_packet.len() - 2;
    connect_packet[1] = remaining_len as u8;

    client.write_all(&connect_packet).await.unwrap();

    // Should receive CONNACK with success
    let mut connack = vec![0u8; 4];
    client.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[0], 0x20); // CONNACK
    assert_eq!(connack[3], 0x00); // ACCEPTED

    let _ = server.stop().await;
}


#[tokio::test]
async fn test_topic_validation() {
    init_test_logging();
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect successfully first
    let mut client = TcpStream::connect(&address).await.unwrap();
    send_connect(&mut client, "validator").await;
    read_connack(&mut client).await;

    // Test 1: Subscribe with invalid wildcard usage
    let mut subscribe_packet = vec![
        0x82, // SUBSCRIBE packet type with QoS=1
        0x00, // Remaining length (will update)
    ];

    // Variable header
    subscribe_packet.extend_from_slice(&[0x00, 0x01]); // Packet ID

    // Payload with invalid topic filter
    subscribe_packet.extend_from_slice(&[0x00, 0x07]); // Topic length
    subscribe_packet.extend_from_slice(b"test/+a"); // Invalid: + not alone in level
    subscribe_packet.push(0x00); // QoS 0

    // Update remaining length
    let remaining_len = subscribe_packet.len() - 2;
    subscribe_packet[1] = remaining_len as u8;

    client.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut header = [0u8; 2];
    client.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x90); // SUBACK

    let remaining_len = header[1] as usize;
    let mut payload = vec![0u8; remaining_len];
    client.read_exact(&mut payload).await.unwrap();

    assert_eq!(payload[0], 0x00); // Packet ID MSB
    assert_eq!(payload[1], 0x01); // Packet ID LSB
    assert_eq!(payload[2], 0x80); // FAILURE

    // Test 2: Subscribe with # not at end
    let mut subscribe_packet = vec![
        0x82, // SUBSCRIBE packet type with QoS=1
        0x00, // Remaining length (will update)
    ];

    // Variable header
    subscribe_packet.extend_from_slice(&[0x00, 0x02]); // Packet ID

    // Payload with invalid topic filter
    subscribe_packet.extend_from_slice(&[0x00, 0x08]); // Topic length
    subscribe_packet.extend_from_slice(b"test/#/a"); // Invalid: # not at end
    subscribe_packet.push(0x00); // QoS 0

    // Update remaining length
    let remaining_len = subscribe_packet.len() - 2;
    subscribe_packet[1] = remaining_len as u8;

    client.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let mut header = [0u8; 2];
    client.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x90); // SUBACK

    let remaining_len = header[1] as usize;
    let mut payload = vec![0u8; remaining_len];
    client.read_exact(&mut payload).await.unwrap();

    assert_eq!(payload[0], 0x00); // Packet ID MSB
    assert_eq!(payload[1], 0x02); // Packet ID LSB
    assert_eq!(payload[2], 0x80); // FAILURE

    let _ = server.stop().await;
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

    // Send CONNECT packet for publisher
    let connect_packet = [
        0x10, // CONNECT packet type
        0x12, // Remaining length = 18
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x3C, // Keep alive = 60 seconds
        0x00, 0x06, // Client ID length
        b'q', b'o', b's', b'p', b'u', b'b', // Client ID "qospub"
    ];

    pub_stream.write_all(&connect_packet).await.unwrap();
    pub_stream.flush().await.unwrap();

    // Read CONNACK for publisher
    let mut response = [0u8; 4];
    pub_stream.read_exact(&mut response).await.unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[3], 0x00); // Return code: accepted

    // Small delay to ensure connection is established
    tokio::time::sleep(Duration::from_millis(10)).await;

    // First test PINGREQ to ensure session is working
    let pingreq_packet = [0xC0, 0x00]; // PINGREQ
    pub_stream.write_all(&pingreq_packet).await.unwrap();
    pub_stream.flush().await.unwrap();

    // Read PINGRESP
    let mut pingresp = [0u8; 2];
    let result = timeout(Duration::from_secs(1), pub_stream.read_exact(&mut pingresp)).await;
    assert!(result.is_ok(), "Timeout waiting for PINGRESP");
    result.unwrap().unwrap();
    assert_eq!(pingresp[0], 0xD0); // PINGRESP packet type
    assert_eq!(pingresp[1], 0x00); // Remaining length

    // Now send PUBLISH with QoS=1
    let publish_packet = [
        0x32, // PUBLISH packet type with QoS=1 (0x30 | 0x02)
        0x10, // Remaining length = 16
        0x00, 0x05, // Topic length
        b't', b'e', b's', b't', b'/', // Topic "test/"
        0x00, 0x42, // Packet ID = 66
        b'h', b'e', b'l', b'l', b'o', // Payload "hello"
    ];

    pub_stream.write_all(&publish_packet).await.unwrap();
    pub_stream.flush().await.unwrap();

    // Read PUBACK
    let mut puback = [0u8; 4];
    let result = timeout(Duration::from_secs(2), pub_stream.read_exact(&mut puback)).await;

    assert!(result.is_ok(), "Timeout waiting for PUBACK");
    result.unwrap().unwrap(); // This just ensures no error

    assert_eq!(puback[0], 0x40); // PUBACK packet type
    assert_eq!(puback[1], 0x02); // Remaining length
    assert_eq!(puback[2], 0x00); // Packet ID MSB
    assert_eq!(puback[3], 0x42); // Packet ID LSB = 66

    // Send DISCONNECT
    let disconnect_packet = [0xE0, 0x00];
    pub_stream.write_all(&disconnect_packet).await.unwrap();

    let _ = server.stop().await;
}

