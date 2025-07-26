use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn test_connection(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing connection to: {}", address);

    let mut stream = TcpStream::connect(address).await?;
    println!("✓ Connected successfully");

    // Send CONNECT packet
    let connect_packet = vec![
        0x10, // CONNECT packet type
        0x0E, // Remaining length (14 bytes)
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x3C, // Keep alive (60 seconds)
        0x00, 0x04, // Client ID length
        b't', b'e', b's', b't', // Client ID "test"
    ];

    stream.write_all(&connect_packet).await?;
    println!("✓ Sent CONNECT packet");

    // Read CONNACK
    let mut buffer = [0u8; 4];
    stream.read_exact(&mut buffer).await?;

    if buffer[0] == 0x20 && buffer[3] == 0x00 {
        println!("✓ Received CONNACK - Connection accepted");
    } else {
        println!("✗ Unexpected response: {:?}", buffer);
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    println!("IoTD IPv6 Connection Test");
    println!("=========================\n");

    // Test different addresses
    let test_addresses = vec![
        ("127.0.0.1:1883", "IPv4 localhost"),
        ("[::1]:1883", "IPv6 localhost"),
        ("localhost:1883", "Hostname (may resolve to IPv4 or IPv6)"),
    ];

    for (address, description) in test_addresses {
        println!("Test: {}", description);
        match test_connection(address).await {
            Ok(_) => println!("✓ Test passed\n"),
            Err(e) => println!("✗ Test failed: {}\n", e),
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("Note: Make sure IoTD is running with appropriate listen address:");
    println!("  IPv4 only:  iotd -l 127.0.0.1:1883");
    println!("  IPv6 only:  iotd -l [::1]:1883");
    println!("  Both:       iotd -l [::]:1883");
}
