use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_two_clients_connect() {
    
    // Start server in background
    let server_handle = tokio::spawn(async {
        let server = iothub::server::IoTHub::new("127.0.0.1:18834".to_string()).await.unwrap();
        server.run().await.unwrap();
    });

    // Connect first client with retry
    let mut client1 = connect_with_retry("127.0.0.1:18834").await;
    println!("Client 1 connected");

    // Send CONNECT for client 1
    send_connect(&mut client1, "client1").await;
    println!("Client 1 sent CONNECT");

    // Read CONNACK for client 1
    read_connack(&mut client1).await;
    println!("Client 1 received CONNACK");

    // Connect second client with retry
    let mut client2 = connect_with_retry("127.0.0.1:18834").await;
    println!("Client 2 connected");

    // Send CONNECT for client 2
    send_connect(&mut client2, "client2").await;
    println!("Client 2 sent CONNECT");

    // Read CONNACK for client 2
    read_connack(&mut client2).await;
    println!("Client 2 received CONNACK");

    println!("Both clients connected successfully!");
    server_handle.abort();
}

async fn connect_with_retry(addr: &str) -> TcpStream {
    for _ in 0..3 { // Try up to 3 times
        match TcpStream::connect(addr).await {
            Ok(stream) => return stream,
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    panic!("Failed to connect to server after 3 retries");
}

async fn send_connect(stream: &mut TcpStream, client_id: &str) {
    let mut connect_packet = vec![
        0x10, // CONNECT packet type
        (12 + client_id.len()) as u8, // Remaining length = 2 (protocol name length) + 4 (MQTT) + 1 (level) + 1 (flags) + 2 (keep alive) + 2 (client ID length) + client_id.len()
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