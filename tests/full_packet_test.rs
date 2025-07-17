use std::error::Error;
use anyhow::Result;
use bytes::BytesMut;
use iothub::config::{Config, ServerConfig};
use iothub::protocol::packet::{self, Packet, QoS, ConnectPacket, PublishPacket, SubscribePacket, UnsubscribePacket};
use iothub::server::Server;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

// ===================================
// Test Setup and Helper Functions
// ===================================

struct TestClient {
    stream: TcpStream,
}

impl TestClient {
    async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self { stream })
    }

    async fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let mut buf = BytesMut::new();
        packet.encode(&mut buf);
        self.stream.write_all(&buf).await?;
        Ok(())
    }

    async fn read_packet_from_stream(&mut self) -> Result<Option<Packet>> {
        match packet::Packet::decode(&mut self.stream).await {
            Ok(packet) => Ok(Some(packet)),
            Err(e) => {
                if let Some(io_error) = e.source().and_then(|e| e.downcast_ref::<std::io::Error>()) {
                    if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Ok(None); // Connection closed cleanly
                    }
                }
                Err(e.into()) // Other errors
            }
        }
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }
}

async fn setup_server() -> (Arc<Server>, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let config = Config {
        server: ServerConfig {
            listen_addresses: vec![format!("tcp://{}", addr)],
            ..Default::default()
        },
        ..Default::default()
    };
    
    let server = Server::new(&config);
    let server_run_clone = Arc::clone(&server);
    
    tokio::spawn(async move {
        server_run_clone.run().await;
    });
    
    tokio::time::sleep(Duration::from_millis(100)).await;

    (server, addr)
}

// ===================================
// Test Cases
// ===================================

#[tokio::test]
async fn test_connect_disconnect_works() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;

    client.send_packet(&Packet::Connect(ConnectPacket {
        protocol_name: "MQTT".to_string(),
        protocol_level: 4,
        clean_session: true,
        keep_alive: 60,
        client_id: "client-1".to_string(),
    })).await?;
    let response = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;
    assert!(matches!(response, Some(Packet::ConnAck(_))), "Expected CONNACK");

    client.send_packet(&Packet::Disconnect).await?;
    let result = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await?;
    assert!(result.is_err(), "Connection should be closed by server");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_publish_works() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;
    client.send_packet(&Packet::Connect(ConnectPacket {
        protocol_name: "MQTT".to_string(),
        protocol_level: 4,
        clean_session: true,
        keep_alive: 60,
        client_id: "client-pub".to_string(),
    })).await?;
    timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;

    let publish_packet = Packet::Publish(PublishPacket {
        topic: "test/topic".to_string(),
        qos: QoS::AtMostOnce,
        payload: "hello".into(),
        packet_id: None,
        retain: false,
        dup: false,
    });
    client.send_packet(&publish_packet).await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_subscribe_unsubscribe_works() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;
    client.send_packet(&Packet::Connect(ConnectPacket {
        protocol_name: "MQTT".to_string(),
        protocol_level: 4,
        clean_session: true,
        keep_alive: 60,
        client_id: "client-sub".to_string(),
    })).await?;
    timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;

    client.send_packet(&Packet::Subscribe(SubscribePacket {
        packet_id: 123,
        topic_filters: vec![("test/topic".to_string(), QoS::AtMostOnce)],
    })).await?;
    let response = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;
    match response {
        Some(Packet::SubAck(p)) => assert_eq!(p.packet_id, 123),
        _ => panic!("Expected SUBACK"),
    }

    client.send_packet(&Packet::Unsubscribe(UnsubscribePacket {
        packet_id: 124,
        topic_filters: vec!["test/topic".to_string()],
    })).await?;
    let response = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;
    match response {
        Some(Packet::UnsubAck(p)) => assert_eq!(p.packet_id, 124),
        _ => panic!("Expected UNSUBACK"),
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_ping_works() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;
    client.send_packet(&Packet::Connect(ConnectPacket {
        protocol_name: "MQTT".to_string(),
        protocol_level: 4,
        clean_session: true,
        keep_alive: 60,
        client_id: "client-ping".to_string(),
    })).await?;
    timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;

    client.send_packet(&Packet::PingReq).await?;
    let response = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await??;
    assert!(matches!(response, Some(Packet::PingResp)), "Expected PINGRESP");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_bad_packet_not_works() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;
    
    client.send_packet(&Packet::Publish(PublishPacket {
        topic: "test/topic".to_string(),
        qos: QoS::AtMostOnce,
        payload: "hello".into(),
        packet_id: None,
        retain: false,
        dup: false,
    })).await?;

    let result = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await?;
    assert!(result.is_err(), "Server should close connection on bad packet");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_malformed_packet_causes_disconnect() -> Result<()> {
    let (server, addr) = setup_server().await;
    let mut client = TestClient::connect(&addr).await?;

    let malformed_packet: &[u8] = &[0x00, 0x02, 0xDE, 0xAD];
    client.send_raw(malformed_packet).await?;

    let result = timeout(Duration::from_secs(2), client.read_packet_from_stream()).await?;
    assert!(result.is_err(), "Server should close connection on malformed packet");

    server.shutdown().await;
    Ok(())
}
