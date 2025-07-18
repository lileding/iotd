use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[tokio::test]
async fn test_basic_connect_and_disconnect() {
    // Start server
    let mut config = iothub::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iothub::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect client
    let mut stream = TcpStream::connect(&address).await.unwrap();

    // Send CONNECT packet (MQTT v3.1.1)
    let connect_packet = [
        0x10, // CONNECT packet type
        0x12, // Remaining length = 18 (12 + 4 + 2)
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
    // Start server
    let mut config = iothub::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iothub::server::start(config).await.unwrap();
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

async fn send_connect(stream: &mut TcpStream, client_id: &str) {
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        (12 + client_id.len()) as u8, // Remaining length
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
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
