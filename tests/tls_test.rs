use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

fn generate_test_certs() -> (tempfile::NamedTempFile, tempfile::NamedTempFile) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();

    let mut cert_file = tempfile::NamedTempFile::new().unwrap();
    cert_file.write_all(cert_pem.as_bytes()).unwrap();

    let mut key_file = tempfile::NamedTempFile::new().unwrap();
    key_file.write_all(key_pem.as_bytes()).unwrap();

    (cert_file, key_file)
}

fn tls_client_config(cert_pem: &[u8]) -> tokio_rustls::TlsConnector {
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    let mut root_store = RootCertStore::empty();
    let certs: Vec<_> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for cert in certs {
        root_store.add(cert).unwrap();
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    tokio_rustls::TlsConnector::from(Arc::new(config))
}

fn make_connect_packet(client_id: &str) -> Vec<u8> {
    let client_id_bytes = client_id.as_bytes();
    let remaining_length = 10 + 2 + client_id_bytes.len();
    let mut packet = Vec::new();
    packet.push(0x10); // CONNECT
    packet.push(remaining_length as u8);
    packet.extend_from_slice(&[0x00, 0x04, b'M', b'Q', b'T', b'T']); // Protocol name
    packet.push(0x04); // Protocol level 3.1.1
    packet.push(0x02); // Clean session
    packet.extend_from_slice(&[0x00, 0x3C]); // Keep alive 60s
    packet.push((client_id_bytes.len() >> 8) as u8);
    packet.push(client_id_bytes.len() as u8);
    packet.extend_from_slice(client_id_bytes);
    packet
}

#[tokio::test]
async fn test_tls_connect() {
    let (cert_file, key_file) = generate_test_certs();
    let cert_pem = std::fs::read(cert_file.path()).unwrap();

    let mut config = iotd::config::Config::default();
    config.listen = vec!["tls://127.0.0.1:0".to_string()];
    config.tls = Some(iotd::config::TlsConfig {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    });

    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();

    // Connect via TLS
    let connector = tls_client_config(&cert_pem);
    let tcp_stream = TcpStream::connect(&address).await.unwrap();
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let mut tls_stream = connector.connect(server_name, tcp_stream).await.unwrap();

    // Send CONNECT
    tls_stream
        .write_all(&make_connect_packet("tlsclient"))
        .await
        .unwrap();
    tls_stream.flush().await.unwrap();

    // Read CONNACK
    let mut response = [0u8; 4];
    timeout(Duration::from_secs(5), tls_stream.read_exact(&mut response))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(response[0], 0x20); // CONNACK
    assert_eq!(response[1], 0x02);
    assert_eq!(response[3], 0x00); // Accepted

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_tls_publish_subscribe() {
    let (cert_file, key_file) = generate_test_certs();
    let cert_pem = std::fs::read(cert_file.path()).unwrap();

    let mut config = iotd::config::Config::default();
    config.listen = vec!["tls://127.0.0.1:0".to_string()];
    config.tls = Some(iotd::config::TlsConfig {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    });

    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();
    let connector = tls_client_config(&cert_pem);
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();

    // Subscriber
    let tcp_stream = TcpStream::connect(&address).await.unwrap();
    let mut subscriber = connector
        .connect(server_name.clone(), tcp_stream)
        .await
        .unwrap();
    subscriber
        .write_all(&make_connect_packet("tlssub"))
        .await
        .unwrap();
    subscriber.flush().await.unwrap();
    let mut connack = [0u8; 4];
    subscriber.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[3], 0x00);

    // Subscribe to "test/tls"
    let subscribe_packet = [
        0x82, // SUBSCRIBE
        0x0D, // Remaining length = 2 + 2 + 8 + 1 = 13
        0x00, 0x01, // Packet ID
        0x00, 0x08, // Topic filter length
        b't', b'e', b's', b't', b'/', b't', b'l', b's', // "test/tls"
        0x00, // QoS 0
    ];
    subscriber.write_all(&subscribe_packet).await.unwrap();
    subscriber.flush().await.unwrap();

    // Read SUBACK
    let mut suback = [0u8; 5];
    subscriber.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90); // SUBACK
    assert_eq!(suback[4], 0x00); // Success QoS 0

    // Publisher
    let tcp_stream = TcpStream::connect(&address).await.unwrap();
    let mut publisher = connector
        .connect(server_name.clone(), tcp_stream)
        .await
        .unwrap();
    publisher
        .write_all(&make_connect_packet("tlspub"))
        .await
        .unwrap();
    publisher.flush().await.unwrap();
    let mut connack = [0u8; 4];
    publisher.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[3], 0x00);

    // Publish to "test/tls"
    let publish_packet = [
        0x30, // PUBLISH QoS 0
        0x0D, // Remaining length
        0x00, 0x08, // Topic length
        b't', b'e', b's', b't', b'/', b't', b'l', b's', // "test/tls"
        b'h', b'i', b'!', // Payload "hi!"
    ];
    publisher.write_all(&publish_packet).await.unwrap();
    publisher.flush().await.unwrap();

    // Subscriber should receive the message
    let mut buf = [0u8; 64];
    let n = timeout(Duration::from_secs(5), subscriber.read(&mut buf))
        .await
        .unwrap()
        .unwrap();

    assert!(n > 0);
    assert_eq!(buf[0] & 0xF0, 0x30); // PUBLISH

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_tcp_and_tls_simultaneous() {
    let (cert_file, key_file) = generate_test_certs();
    let cert_pem = std::fs::read(cert_file.path()).unwrap();

    let mut config = iotd::config::Config::default();
    config.listen = vec![
        "tcp://127.0.0.1:0".to_string(),
        "tls://127.0.0.1:0".to_string(),
    ];
    config.tls = Some(iotd::config::TlsConfig {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    });

    let server = iotd::server::start(config).await.unwrap();
    let addresses = server.addresses().await;
    assert_eq!(addresses.len(), 2);
    let tcp_addr = &addresses[0];
    let tls_addr = &addresses[1];

    // TCP subscriber
    let mut tcp_sub = TcpStream::connect(tcp_addr).await.unwrap();
    tcp_sub
        .write_all(&make_connect_packet("tcpsub"))
        .await
        .unwrap();
    tcp_sub.flush().await.unwrap();
    let mut connack = [0u8; 4];
    tcp_sub.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[3], 0x00);

    // Subscribe via TCP
    // Subscribe to "cross/tls" (remaining length = 2 + 2 + 9 + 1 = 14)
    let subscribe_packet = [
        0x82, 0x0E, 0x00, 0x01, 0x00, 0x09, b'c', b'r', b'o', b's', b's', b'/', b't', b'l', b's',
        0x00,
    ];
    tcp_sub.write_all(&subscribe_packet).await.unwrap();
    tcp_sub.flush().await.unwrap();
    let mut suback = [0u8; 5];
    tcp_sub.read_exact(&mut suback).await.unwrap();
    assert_eq!(suback[0], 0x90);

    // TLS publisher
    let connector = tls_client_config(&cert_pem);
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tcp_stream = TcpStream::connect(tls_addr).await.unwrap();
    let mut tls_pub = connector.connect(server_name, tcp_stream).await.unwrap();
    tls_pub
        .write_all(&make_connect_packet("tlspub"))
        .await
        .unwrap();
    tls_pub.flush().await.unwrap();
    let mut connack = [0u8; 4];
    tls_pub.read_exact(&mut connack).await.unwrap();
    assert_eq!(connack[3], 0x00);

    // Publish via TLS to "cross/tls"
    let publish_packet = [
        0x30, // PUBLISH QoS 0
        0x0E, // Remaining length
        0x00, 0x09, // Topic length
        b'c', b'r', b'o', b's', b's', b'/', b't', b'l', b's', // "cross/tls"
        b'x', b'y', b'z', // Payload "xyz"
    ];
    tls_pub.write_all(&publish_packet).await.unwrap();
    tls_pub.flush().await.unwrap();

    // TCP subscriber should receive the cross-protocol message
    let mut buf = [0u8; 64];
    let n = timeout(Duration::from_secs(5), tcp_sub.read(&mut buf))
        .await
        .unwrap()
        .unwrap();

    assert!(n > 0);
    assert_eq!(buf[0] & 0xF0, 0x30); // PUBLISH

    let _ = server.stop().await;
}

#[tokio::test]
async fn test_tls_missing_config() {
    let mut config = iotd::config::Config::default();
    config.listen = vec!["tls://127.0.0.1:0".to_string()];
    // No TLS config set

    match iotd::server::start(config).await {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("tls") || msg.contains("TLS"),
                "Error should mention TLS: {msg}"
            );
        }
        Ok(_) => panic!("Expected startup failure when TLS config is missing"),
    }
}

#[tokio::test]
async fn test_tls_invalid_cert_path() {
    let mut config = iotd::config::Config::default();
    config.listen = vec!["tls://127.0.0.1:0".to_string()];
    config.tls = Some(iotd::config::TlsConfig {
        cert_file: "/nonexistent/cert.pem".into(),
        key_file: "/nonexistent/key.pem".into(),
    });

    assert!(iotd::server::start(config).await.is_err());
}
