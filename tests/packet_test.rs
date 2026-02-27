use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Helper function to start test server and return server with address
async fn start_test_server() -> (iotd::server::Server, String) {
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();
    (server, address)
}

/// Helper function to connect to test server with retries
async fn connect_to_server(address: &str) -> TcpStream {
    for _ in 0..5 {
        match TcpStream::connect(address).await {
            Ok(stream) => return stream,
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
    panic!("Failed to connect to test server after retries");
}

/// Helper function to create CONNECT packet
fn create_connect_packet(client_id: &str, clean_session: bool, keep_alive: u16) -> Vec<u8> {
    let mut packet = vec![
        0x10, // CONNECT packet type
        0x00, // Will be filled with remaining length
        0x00,
        0x04, // Protocol name length
        b'M',
        b'Q',
        b'T',
        b'T',                                    // Protocol name "MQTT"
        0x04,                                    // Protocol level (3.1.1)
        if clean_session { 0x02 } else { 0x00 }, // Connect flags
        (keep_alive >> 8) as u8,
        keep_alive as u8, // Keep alive
        (client_id.len() >> 8) as u8,
        client_id.len() as u8, // Client ID length
    ];
    packet.extend_from_slice(client_id.as_bytes());

    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

/// Helper function to create PUBLISH packet
fn create_publish_packet(topic: &str, payload: &[u8], qos: u8, retain: bool) -> Vec<u8> {
    let mut flags = 0x30; // PUBLISH packet type
    if retain {
        flags |= 0x01;
    }
    flags |= (qos & 0x03) << 1;

    let mut packet = vec![flags];
    let mut remaining_data = Vec::new();

    // Topic name
    remaining_data.extend_from_slice(&[(topic.len() >> 8) as u8, topic.len() as u8]);
    remaining_data.extend_from_slice(topic.as_bytes());

    // Payload
    remaining_data.extend_from_slice(payload);

    // Remaining length
    packet.push(remaining_data.len() as u8);
    packet.extend_from_slice(&remaining_data);
    packet
}

/// Helper function to create SUBSCRIBE packet
fn create_subscribe_packet(packet_id: u16, topic: &str, qos: u8) -> Vec<u8> {
    let mut packet = vec![
        0x82, // SUBSCRIBE packet type with flags
        0x00, // Will be filled with remaining length
        (packet_id >> 8) as u8,
        packet_id as u8, // Packet ID
        (topic.len() >> 8) as u8,
        topic.len() as u8, // Topic length
    ];
    packet.extend_from_slice(topic.as_bytes());
    packet.push(qos); // Requested QoS

    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

/// Helper function to create UNSUBSCRIBE packet
fn create_unsubscribe_packet(packet_id: u16, topic: &str) -> Vec<u8> {
    let mut packet = vec![
        0xA2, // UNSUBSCRIBE packet type with flags
        0x00, // Will be filled with remaining length
        (packet_id >> 8) as u8,
        packet_id as u8, // Packet ID
        (topic.len() >> 8) as u8,
        topic.len() as u8, // Topic length
    ];
    packet.extend_from_slice(topic.as_bytes());

    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

/// Helper function to create PINGREQ packet
fn create_pingreq_packet() -> Vec<u8> {
    vec![0xC0, 0x00] // PINGREQ with 0 remaining length
}

/// Helper function to create DISCONNECT packet
fn create_disconnect_packet() -> Vec<u8> {
    vec![0xE0, 0x00] // DISCONNECT with 0 remaining length
}

/// Helper function to read packet with timeout
async fn read_packet_with_timeout(
    stream: &mut TcpStream,
    timeout_duration: Duration,
) -> Result<Vec<u8>, String> {
    // Read fixed header
    let mut fixed_header = [0u8; 2];
    match timeout(timeout_duration, stream.read_exact(&mut fixed_header)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(format!("IO error reading fixed header: {}", e)),
        Err(_) => return Err("Timeout reading fixed header".to_string()),
    }

    let remaining_length = fixed_header[1] as usize;
    let mut packet = fixed_header.to_vec();

    if remaining_length > 0 {
        let mut remaining_data = vec![0u8; remaining_length];
        match timeout(timeout_duration, stream.read_exact(&mut remaining_data)).await {
            Ok(Ok(_)) => packet.extend_from_slice(&remaining_data),
            Ok(Err(e)) => return Err(format!("IO error reading remaining data: {}", e)),
            Err(_) => return Err("Timeout reading remaining data".to_string()),
        }
    }

    Ok(packet)
}

// =============================================================================
// CONNECT/CONNACK Tests
// =============================================================================

#[tokio::test]
async fn test_connect_with_client_id() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Send CONNECT with client ID
    let connect_packet = create_connect_packet("testclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();

    // Read CONNACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[1], 0x02); // Remaining length
    assert_eq!(response[2], 0x00); // Session present = false
    assert_eq!(response[3], 0x00); // Return code = accepted
}

#[tokio::test]
async fn test_connect_without_client_id() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Send CONNECT without client ID (empty string)
    let connect_packet = create_connect_packet("", true, 60);
    stream.write_all(&connect_packet).await.unwrap();

    // Read CONNACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[3], 0x00); // Return code = accepted (server should generate client ID)
}

#[tokio::test]
async fn test_connect_clean_session_flag() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Send CONNECT with clean_session = false
    let connect_packet = create_connect_packet("testclient", false, 60);
    stream.write_all(&connect_packet).await.unwrap();

    // Read CONNACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x20); // CONNACK packet type
    assert_eq!(response[3], 0x00); // Return code = accepted
}

#[tokio::test]
async fn test_connect_duplicate_client_id() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // First connection
    let mut stream1 = connect_to_server(&address).await;
    let connect_packet = create_connect_packet("duplicateclient", true, 60);
    stream1.write_all(&connect_packet).await.unwrap();
    let _response1 = read_packet_with_timeout(&mut stream1, Duration::from_secs(2))
        .await
        .unwrap();

    // Second connection with same client ID
    let mut stream2 = connect_to_server(&address).await;
    stream2.write_all(&connect_packet).await.unwrap();
    let response2 = read_packet_with_timeout(&mut stream2, Duration::from_secs(2))
        .await
        .unwrap();

    // Both should get CONNACK, but first connection should be closed
    assert_eq!(response2[0], 0x20); // CONNACK packet type
    assert_eq!(response2[3], 0x00); // Return code = accepted
}

// =============================================================================
// PUBLISH Tests
// =============================================================================

#[tokio::test]
async fn test_publish_qos0() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("publisher", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send PUBLISH
    let publish_packet = create_publish_packet("test/topic", b"hello world", 0, false);
    stream.write_all(&publish_packet).await.unwrap();

    // For QoS 0, no response expected
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_publish_retained() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("publisher", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send PUBLISH with retain flag
    let publish_packet = create_publish_packet("test/retained", b"retained message", 0, true);
    stream.write_all(&publish_packet).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_publish_empty_payload() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("publisher", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send PUBLISH with empty payload
    let publish_packet = create_publish_packet("test/empty", b"", 0, false);
    stream.write_all(&publish_packet).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
}

// =============================================================================
// SUBSCRIBE/SUBACK Tests
// =============================================================================

#[tokio::test]
async fn test_subscribe_single_topic() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("subscriber", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send SUBSCRIBE
    let subscribe_packet = create_subscribe_packet(1, "test/topic", 0);
    stream.write_all(&subscribe_packet).await.unwrap();

    // Read SUBACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x90); // SUBACK packet type
    assert_eq!(response[1], 0x03); // Remaining length
    assert_eq!(response[2], 0x00); // Packet ID high byte
    assert_eq!(response[3], 0x01); // Packet ID low byte
    assert_eq!(response[4], 0x00); // Return code (Maximum QoS 0)
}

#[tokio::test]
async fn test_subscribe_wildcard_topics() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("subscriber", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Subscribe to single-level wildcard
    let subscribe_packet = create_subscribe_packet(1, "test/+", 0);
    stream.write_all(&subscribe_packet).await.unwrap();
    let _suback = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Subscribe to multi-level wildcard
    let subscribe_packet = create_subscribe_packet(2, "test/#", 0);
    stream.write_all(&subscribe_packet).await.unwrap();
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x90); // SUBACK packet type
    assert_eq!(response[4], 0x00); // Return code (Maximum QoS 0)
}

#[tokio::test]
async fn test_subscribe_invalid_topic() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("subscriber", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Subscribe to invalid topic (contains null character)
    let subscribe_packet = create_subscribe_packet(1, "test/\0/invalid", 0);
    stream.write_all(&subscribe_packet).await.unwrap();

    // Should still get SUBACK but with failure code
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0x90); // SUBACK packet type
                                   // Return code might be 0x80 (failure) depending on implementation
}

// =============================================================================
// UNSUBSCRIBE/UNSUBACK Tests
// =============================================================================

#[tokio::test]
async fn test_unsubscribe() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("subscriber", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Subscribe first
    let subscribe_packet = create_subscribe_packet(1, "test/topic", 0);
    stream.write_all(&subscribe_packet).await.unwrap();
    let _suback = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Now unsubscribe
    let unsubscribe_packet = create_unsubscribe_packet(2, "test/topic");
    stream.write_all(&unsubscribe_packet).await.unwrap();

    // Read UNSUBACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0xB0); // UNSUBACK packet type
    assert_eq!(response[1], 0x02); // Remaining length
    assert_eq!(response[2], 0x00); // Packet ID high byte
    assert_eq!(response[3], 0x02); // Packet ID low byte
}

#[tokio::test]
async fn test_unsubscribe_nonexistent_topic() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("subscriber", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Unsubscribe from topic we never subscribed to
    let unsubscribe_packet = create_unsubscribe_packet(1, "nonexistent/topic");
    stream.write_all(&unsubscribe_packet).await.unwrap();

    // Should still get UNSUBACK
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0xB0); // UNSUBACK packet type
    assert_eq!(response[1], 0x02); // Remaining length
}

// =============================================================================
// PINGREQ/PINGRESP Tests
// =============================================================================

#[tokio::test]
async fn test_pingreq_pingresp() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("pingclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send PINGREQ
    let pingreq_packet = create_pingreq_packet();
    stream.write_all(&pingreq_packet).await.unwrap();

    // Read PINGRESP
    let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response[0], 0xD0); // PINGRESP packet type
    assert_eq!(response[1], 0x00); // Remaining length = 0
}

#[tokio::test]
async fn test_multiple_pingreq() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("pingclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send multiple PINGREQ packets
    for _ in 0..3 {
        let pingreq_packet = create_pingreq_packet();
        stream.write_all(&pingreq_packet).await.unwrap();

        let response = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(response[0], 0xD0); // PINGRESP packet type
        assert_eq!(response[1], 0x00); // Remaining length = 0
    }
}

// =============================================================================
// DISCONNECT Tests
// =============================================================================

#[tokio::test]
async fn test_disconnect() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("disconnectclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send DISCONNECT
    let disconnect_packet = create_disconnect_packet();
    stream.write_all(&disconnect_packet).await.unwrap();

    // No response expected for DISCONNECT
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should be closed
    let mut buf = [0u8; 1];
    match stream.read(&mut buf).await {
        Ok(0) => {} // Connection closed
        Ok(_) => panic!("Expected connection to be closed"),
        Err(_) => {} // Connection error, which is expected
    }
}

// =============================================================================
// Invalid Packet Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_packet_type() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("invalidclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send invalid packet type (0x00 is reserved)
    let invalid_packet = vec![0x00, 0x02, 0x00, 0x00];
    stream.write_all(&invalid_packet).await.unwrap();

    // Connection should be closed due to protocol error
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_malformed_packet() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("malformedclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send malformed PUBLISH packet (remaining length doesn't match actual data)
    let malformed_packet = vec![0x30, 0x10, 0x00, 0x04]; // Says 16 bytes remaining, but only 4 provided
    stream.write_all(&malformed_packet).await.unwrap();

    // Connection should be closed due to malformed packet
    tokio::time::sleep(Duration::from_millis(100)).await;
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[tokio::test]
async fn test_packet_before_connect() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Send PUBLISH before CONNECT (should be rejected)
    let publish_packet = create_publish_packet("test/topic", b"hello", 0, false);
    stream.write_all(&publish_packet).await.unwrap();

    // Connection should be closed due to protocol violation
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_large_packet() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = connect_to_server(&address).await;

    // Connect first
    let connect_packet = create_connect_packet("largeclient", true, 60);
    stream.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut stream, Duration::from_secs(2))
        .await
        .unwrap();

    // Send large PUBLISH packet
    let large_payload = vec![b'A'; 1024]; // 1KB payload
    let publish_packet = create_publish_packet("test/large", &large_payload, 0, false);
    stream.write_all(&publish_packet).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
}

// =============================================================================
// Message Routing Tests (Future Implementation)
// =============================================================================

#[tokio::test]
async fn test_publish_subscribe_routing() {
    let (_server, address) = start_test_server().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create subscriber
    let mut subscriber = connect_to_server(&address).await;
    let connect_packet = create_connect_packet("subscriber", true, 60);
    subscriber.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut subscriber, Duration::from_secs(2))
        .await
        .unwrap();

    // Subscribe to topic
    let subscribe_packet = create_subscribe_packet(1, "test/routing", 0);
    subscriber.write_all(&subscribe_packet).await.unwrap();
    let _suback = read_packet_with_timeout(&mut subscriber, Duration::from_secs(2))
        .await
        .unwrap();

    // Create publisher
    let mut publisher = connect_to_server(&address).await;
    let connect_packet = create_connect_packet("publisher", true, 60);
    publisher.write_all(&connect_packet).await.unwrap();
    let _connack = read_packet_with_timeout(&mut publisher, Duration::from_secs(2))
        .await
        .unwrap();

    // Publish message
    let publish_packet = create_publish_packet("test/routing", b"routed message", 0, false);
    publisher.write_all(&publish_packet).await.unwrap();

    // TODO: Once message routing is implemented, subscriber should receive the message
    // let received_message = read_packet_with_timeout(&mut subscriber, Duration::from_secs(2)).await.unwrap();
    // assert_eq!(received_message[0] & 0xF0, 0x30); // PUBLISH packet type
}
