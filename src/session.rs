use crate::transport::AsyncStream;
use crate::server::Server;
use crate::broker::Broker;
use crate::protocol::packet;
use anyhow::Result;
use std::sync::Arc;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use uuid::Uuid;

pub struct Session {
    session_id: RwLock<String>,
    clean_session: bool,
    keep_alive: u16,
    stream: RwLock<Box<dyn AsyncStream>>,
    server: Arc<Server>,
    broker: Arc<Broker>,
    shutdown_token: CancellationToken,
    completed_token: CancellationToken,
    connected: AtomicBool,
    message_rx: RwLock<Option<mpsc::Receiver<bytes::Bytes>>>,
    message_tx: mpsc::Sender<bytes::Bytes>,
}

impl Session {
    pub fn new(
        stream: Box<dyn AsyncStream>,
        server: Arc<Server>,
        broker: Arc<Broker>,
    ) -> Arc<Self> {
        let uuid = Uuid::new_v4().to_string();
        let session_id = format!("__anon_{}", uuid);
        let (message_tx, message_rx) = mpsc::channel(1000);
        
        Arc::new(Self {
            session_id: RwLock::new(session_id),
            clean_session: true,
            keep_alive: 0,
            stream: RwLock::new(stream),
            server: Arc::clone(&server),
            broker: Arc::clone(&broker),
            shutdown_token: CancellationToken::new(),
            completed_token: CancellationToken::new(),
            connected: AtomicBool::new(false),
            message_rx: RwLock::new(Some(message_rx)),
            message_tx,
        })
    }

    pub async fn session_id(&self) -> String {
        self.session_id.read().await.clone()
    }

    pub fn message_sender(&self) -> mpsc::Sender<bytes::Bytes> {
        self.message_tx.clone()
    }

    pub async fn run(self: Arc<Self>) {
        let session_id = self.session_id().await;
        info!("Session {} starting", session_id);

        // Get the message receiver
        let message_rx_result = self.message_rx.write().await.take();
        let mut message_rx = match message_rx_result {
            Some(rx) => rx,
            None => {
                error!("Message receiver already taken for session {}", session_id);
                self.cleanup_and_exit().await;
                return;
            }
        };

        // Session handling loop
        loop {
            tokio::select! {
                // Handle incoming packets from client
                result = self.handle_client_packet() => {
                    match result {
                        Ok(should_continue) => {
                            if !should_continue {
                                info!("Session {} ending normally", session_id);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Session {} error: {}", session_id, e);
                            break;
                        }
                    }
                }
                
                // Handle messages from subscribed topics
                Some(message) = message_rx.recv() => {
                    if let Err(e) = self.handle_message(message).await {
                        error!("Failed to handle message for session {}: {}", session_id, e);
                        break;
                    }
                }
                
                // Handle shutdown signal
                _ = self.shutdown_token.cancelled() => {
                    info!("Session {} shutting down", session_id);
                    break;
                }
            }
        }

        // Cleanup and exit
        self.cleanup_and_exit().await;
        info!("Session {} completed", session_id);
    }

    async fn cleanup_and_exit(self: &Arc<Self>) {
        let session_id = self.session_id().await;
        
        // Remove from half-connected sessions only if not connected yet
        if !self.connected.load(Ordering::Acquire) {
            self.broker.remove_half_connected_session(&session_id).await;
        }
        
        // Unregister from server
        self.server.unregister_session(&session_id).await;
        
        // Close the stream
        if let Err(e) = self.stream.write().await.close().await {
            error!("Failed to close stream for session {}: {}", session_id, e);
        }

        // Send completed notification
        self.completed_token.cancel();
    }

    pub async fn shutdown(&self) {
        self.shutdown_token.cancel();
        self.completed_token.cancelled().await;
    }

    async fn handle_client_packet(self: &Arc<Self>) -> Result<bool> {
        // Read and parse MQTT packet from stream
        let mut stream = self.stream.write().await;
        
        // Parse and validate packet directly from stream
        match packet::Packet::decode(&mut *stream).await {
            Ok(packet) => {
                // Handle different packet types
                match packet {
                    packet::Packet::Connect(p) => self.handle_connect(p, stream.as_mut()).await?,
                    packet::Packet::Publish(p) => self.handle_publish(p).await?,
                    packet::Packet::Subscribe(p) => self.handle_subscribe(p, stream.as_mut()).await?,
                    packet::Packet::Unsubscribe(p) => self.handle_unsubscribe(p, stream.as_mut()).await?,
                    packet::Packet::PingReq => self.handle_pingreq(stream.as_mut()).await?,
                    packet::Packet::Disconnect => {
                        self.handle_disconnect().await?;
                        return Ok(false); // End session
                    }
                    _ => {
                        let session_id = self.session_id().await;
                        error!("Unhandled packet type for session {}", session_id);
                    }
                }
            }
            Err(e) => {
                // Check if this is a connection closed error
                if let Some(io_error) = e.source().and_then(|e| e.downcast_ref::<std::io::Error>()) {
                    if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                        let session_id = self.session_id().await;
                        info!("Client {} closed connection", session_id);
                        return Ok(false);
                    }
                }
                let session_id = self.session_id().await;
                error!("Packet decoding error for session {}: {}", session_id, e);
                return Err(e.into());
            }
        }

        Ok(true) // Continue session
    }

    async fn handle_connect(self: &Arc<Self>, packet: packet::ConnectPacket, stream: &mut dyn AsyncStream) -> Result<()> {
        info!("CONNECT received: client_id={}, clean_session={}", packet.client_id, packet.clean_session);

        let old_session_id = self.session_id().await;

        // Update sessionId if clientId is provided
        if !packet.client_id.is_empty() {
            let new_session_id = format!("__client_{}", packet.client_id);
            *self.session_id.write().await = new_session_id;
        }

        // Set connected flag
        self.connected.store(true, Ordering::Release);

        // Send CONNACK response
        let connack = packet::ConnAckPacket {
            session_present: false,
            return_code: crate::protocol::v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        stream.write_all(&buf).await?;

        // Remove from half-connected sessions before registering with server
        self.broker.remove_half_connected_session(&old_session_id).await;

        // Register session with server after CONNACK is sent
        self.server.register_session(Arc::clone(self)).await?;

        Ok(())
    }

    async fn handle_publish(&self, packet: packet::PublishPacket) -> Result<()> {
        // Check if connected
        if !self.connected.load(Ordering::Acquire) {
            let session_id = self.session_id().await;
            error!("PUBLISH received for session {} but not connected", session_id);
            return Ok(());
        }

        let session_id = self.session_id().await;
        info!("PUBLISH received for session {}: topic={}, qos={:?}", session_id, packet.topic, packet.qos);
        // Here you would route the message through the server's router
        // self.server.route_message(packet).await?;
        Ok(())
    }

    async fn handle_subscribe(&self, packet: packet::SubscribePacket, stream: &mut dyn AsyncStream) -> Result<()> {
        // Check if connected
        if !self.connected.load(Ordering::Acquire) {
            let session_id = self.session_id().await;
            error!("SUBSCRIBE received for session {} but not connected", session_id);
            return Ok(());
        }

        let session_id = self.session_id().await;
        info!("SUBSCRIBE received for session {}: topics={:?}", session_id, packet.topic_filters);
        // Handle subscription logic and send SUBACK
        let suback = packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes: vec![crate::protocol::v3::subscribe_return_codes::MAXIMUM_QOS_0; packet.topic_filters.len()],
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::SubAck(suback).encode(&mut buf);
        stream.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_pingreq(&self, stream: &mut dyn AsyncStream) -> Result<()> {
        // Check if connected
        if !self.connected.load(Ordering::Acquire) {
            let session_id = self.session_id().await;
            error!("PINGREQ received for session {} but not connected", session_id);
            return Ok(());
        }

        let session_id = self.session_id().await;
        info!("PINGREQ received for session {}", session_id);
        let pingresp = packet::Packet::PingResp;
        let mut buf = bytes::BytesMut::new();
        pingresp.encode(&mut buf);
        stream.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_unsubscribe(&self, packet: packet::UnsubscribePacket, stream: &mut dyn AsyncStream) -> Result<()> {
        // Check if connected
        if !self.connected.load(Ordering::Acquire) {
            let session_id = self.session_id().await;
            error!("UNSUBSCRIBE received for session {} but not connected", session_id);
            return Ok(());
        }

        let session_id = self.session_id().await;
        info!("UNSUBSCRIBE received for session {}: topics={:?}", session_id, packet.topic_filters);
        // Handle unsubscription logic and send UNSUBACK
        let unsuback = packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::UnsubAck(unsuback).encode(&mut buf);
        stream.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_disconnect(&self) -> Result<()> {
        let session_id = self.session_id().await;
        info!("DISCONNECT received for session {}", session_id);
        // Cancel the session
        self.shutdown_token.cancel();
        Ok(())
    }

    async fn handle_message(&self, message: bytes::Bytes) -> Result<()> {
        // Forward the message and encode it as a packet to AsyncWrite of the stream
        let session_id = self.session_id().await;
        info!("Forwarding message to session {}", session_id);
        self.stream.write().await.write_all(&message).await?;
        Ok(())
    }


    pub fn set_clean_session(&mut self, clean_session: bool) {
        self.clean_session = clean_session;
    }

    pub fn set_keep_alive(&mut self, keep_alive: u16) {
        self.keep_alive = keep_alive;
    }
}
