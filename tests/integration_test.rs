use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

static INIT: std::sync::Once = std::sync::Once::new();

pub fn init_test_logging() {
    INIT.call_once(|| {
        //return;
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
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
