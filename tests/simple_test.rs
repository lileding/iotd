use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[tokio::test]
async fn test_simple_connect() {
    // Start server
    let mut config = iotd::config::Config::default();
    config.server.address = "127.0.0.1:0".to_string();
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connect client
    let mut stream = TcpStream::connect(&address).await.unwrap();
    println!("Connected to server");

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
    println!("Sent CONNECT packet");

    // Read CONNACK with debug
    let mut response = [0u8; 4];
    match timeout(Duration::from_secs(5), stream.read_exact(&mut response)).await {
        Ok(Ok(_)) => {
            println!("Received CONNACK: {:?}", response);
            assert_eq!(response[0], 0x20); // CONNACK packet type
            assert_eq!(response[1], 0x02); // Remaining length
            assert_eq!(response[2], 0x00); // Session present = false
            assert_eq!(response[3], 0x00); // Return code = accepted
        }
        Ok(Err(e)) => panic!("Failed to read CONNACK: {}", e),
        Err(_) => panic!("Timeout reading CONNACK"),
    }

    println!("Test passed!");
    let _ = server.stop().await;
}
