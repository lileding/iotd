use anyhow::Result;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use crate::config::TlsConfig;

#[async_trait]
pub trait AsyncListener: Send + Sync {
    async fn accept(&self) -> Result<Box<dyn AsyncStream>>;
    async fn local_addr(&self) -> Result<SocketAddr>;
    async fn close(&self) -> Result<()>;
}

#[async_trait]
pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Sync + Unpin {
    async fn close(&mut self) -> Result<()>;
    fn peer_addr(&self) -> Result<SocketAddr>;
    fn into_split(
        self: Box<Self>,
    ) -> (
        Box<dyn AsyncRead + Send + Sync + Unpin>,
        Box<dyn AsyncWrite + Send + Sync + Unpin>,
    );
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    TcpTls,
    #[allow(dead_code)]
    WebSocket,
    #[allow(dead_code)]
    WebSocketSecure,
    #[allow(dead_code)]
    Unix,
}

impl Protocol {
    pub fn from_url(url: &str) -> Result<(Self, String)> {
        if let Some(addr) = url.strip_prefix("tcp://") {
            Ok((Protocol::Tcp, addr.to_string()))
        } else if let Some(addr) = url.strip_prefix("tls://") {
            Ok((Protocol::TcpTls, addr.to_string()))
        } else {
            // Bare address without protocol prefix treated as TCP
            Ok((Protocol::Tcp, url.to_string()))
        }
    }
}

// --- TCP ---

pub struct TcpAsyncListener {
    listener: TcpListener,
}

impl TcpAsyncListener {
    pub async fn bind(addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }
}

#[async_trait]
impl AsyncListener for TcpAsyncListener {
    async fn accept(&self) -> Result<Box<dyn AsyncStream>> {
        let (stream, _) = self.listener.accept().await?;
        Ok(Box::new(TcpAsyncStream { stream }))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    async fn close(&self) -> Result<()> {
        // TCP listeners don't need explicit closing in Tokio
        Ok(())
    }
}

pub struct TcpAsyncStream {
    stream: TcpStream,
}

impl AsyncRead for TcpAsyncStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for TcpAsyncStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[async_trait]
impl AsyncStream for TcpAsyncStream {
    async fn close(&mut self) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        self.stream.shutdown().await?;
        Ok(())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.stream.peer_addr()?)
    }

    fn into_split(
        self: Box<Self>,
    ) -> (
        Box<dyn AsyncRead + Send + Sync + Unpin>,
        Box<dyn AsyncWrite + Send + Sync + Unpin>,
    ) {
        let (r, w) = tokio::io::split(self.stream);
        (Box::new(r), Box::new(w))
    }
}

// --- TLS ---

pub struct TlsAsyncListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsAsyncListener {
    pub async fn bind(addr: &str, acceptor: TlsAcceptor) -> Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener, acceptor })
    }
}

#[async_trait]
impl AsyncListener for TlsAsyncListener {
    async fn accept(&self) -> Result<Box<dyn AsyncStream>> {
        let (tcp_stream, _) = self.listener.accept().await?;
        let tls_stream = self.acceptor.accept(tcp_stream).await?;
        Ok(Box::new(TlsAsyncStream { stream: tls_stream }))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct TlsAsyncStream {
    stream: TlsStream<TcpStream>,
}

impl AsyncRead for TlsAsyncStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsAsyncStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[async_trait]
impl AsyncStream for TlsAsyncStream {
    async fn close(&mut self) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        self.stream.shutdown().await?;
        Ok(())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let (tcp_stream, _) = self.stream.get_ref();
        Ok(tcp_stream.peer_addr()?)
    }

    fn into_split(
        self: Box<Self>,
    ) -> (
        Box<dyn AsyncRead + Send + Sync + Unpin>,
        Box<dyn AsyncWrite + Send + Sync + Unpin>,
    ) {
        let (r, w) = tokio::io::split(self.stream);
        (Box::new(r), Box::new(w))
    }
}

// --- TLS helper ---

pub fn build_tls_acceptor(tls_config: &TlsConfig) -> Result<TlsAcceptor> {
    use rustls_pemfile::{certs, private_key};
    use std::fs::File;
    use std::io::BufReader;
    use tokio_rustls::rustls::ServerConfig;

    let cert_file = File::open(&tls_config.cert_file)?;
    let key_file = File::open(&tls_config.key_file)?;

    let certs: Vec<_> =
        certs(&mut BufReader::new(cert_file)).collect::<std::result::Result<Vec<_>, _>>()?;

    let key = private_key(&mut BufReader::new(key_file))?.ok_or_else(|| {
        anyhow::anyhow!("No private key found in {}", tls_config.key_file.display())
    })?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_from_url_tcp() {
        let (protocol, addr) = Protocol::from_url("tcp://0.0.0.0:1883").unwrap();
        assert!(matches!(protocol, Protocol::Tcp));
        assert_eq!(addr, "0.0.0.0:1883");
    }

    #[test]
    fn test_protocol_from_url_tls() {
        let (protocol, addr) = Protocol::from_url("tls://0.0.0.0:8883").unwrap();
        assert!(matches!(protocol, Protocol::TcpTls));
        assert_eq!(addr, "0.0.0.0:8883");
    }

    #[test]
    fn test_protocol_from_url_bare_address() {
        let (protocol, addr) = Protocol::from_url("127.0.0.1:1883").unwrap();
        assert!(matches!(protocol, Protocol::Tcp));
        assert_eq!(addr, "127.0.0.1:1883");
    }
}
