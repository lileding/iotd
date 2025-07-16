use async_trait::async_trait;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use anyhow::Result;

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
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    #[allow(dead_code)]
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
        } else {
            anyhow::bail!("Unsupported protocol: {}", url)
        }
    }
}

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
}