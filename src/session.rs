use crate::transport::AsyncStream;
use crate::server::Server;
use crate::protocol::packet::Packet;
use anyhow::Result;
use std::sync::Arc;
use std::error::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Notify, RwLock};
use tracing::{error, info};
use uuid::Uuid;

pub struct Session {
    session_id: String,
    client_id: RwLock<String>,
    clean_session: bool,
    keep_alive: u16,
    stream: RwLock<Box<dyn AsyncStream>>,
    server: Arc<Server>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
    connected: RwLock<bool>,
    message_rx: RwLock<Option<mpsc::Receiver<bytes::Bytes>>>,
    message_tx: mpsc::Sender<bytes::Bytes>,
}

impl Session {
    pub fn new(
        stream: Box<dyn AsyncStream>,
        server: Arc<Server>,
    ) -> Arc<Self> {
        let session_id = Uuid::new_v4().to_string();
        let client_id = format!("__anon_{}", session_id);
        let (message_tx, message_rx) = mpsc::channel(1000);
        
        Arc::new(Self {
            session_id,
            client_id: RwLock::new(client_id),
            clean_session: true,
            keep_alive: 0,
            stream: RwLock::new(stream),
            server: Arc::clone(&server),
            shutdown: Arc::new(Notify::new()),
            completed: Arc::new(Notify::new()),
            connected: RwLock::new(false),
            message_rx: RwLock::new(Some(message_rx)),
            message_tx,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub async fn client_id(&self) -> String {
        self.client_id.read().await.clone()
    }

    pub fn message_sender(&self) -> mpsc::Sender<bytes::Bytes> {
        self.message_tx.clone()
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Session {} starting", self.session_id);

        // Get the message receiver
        let mut message_rx = self.message_rx.write().await.take()
            .ok_or_else(|| anyhow::anyhow!("Message receiver already taken"))?;

        // Session handling loop
        loop {
            tokio::select! {
                // Handle incoming packets from client
                result = self.handle_client_packet() => {
                    match result {
                        Ok(should_continue) => {
                            if !should_continue {
                                info!("Session {} ending normally", self.session_id);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Session {} error: {}", self.session_id, e);
                            break;
                        }
                    }
                }
                
                // Handle messages from subscribed topics
                Some(message) = message_rx.recv() => {
                    if let Err(e) = self.handle_message(message).await {
                        error!("Failed to handle message for session {}: {}", self.session_id, e);
                    }
                }
                
                // Handle shutdown signal
                _ = self.shutdown.notified() => {
                    info!("Session {} shutting down", self.session_id);
                    break;
                }
            }
        }

        // Cleanup: unregister session from server
        self.server.unregister_session(&self.session_id).await;
        
        // Close the stream
        if let Err(e) = self.stream.write().await.close().await {
            error!("Failed to close stream for session {}: {}", self.session_id, e);
        }

        // Send completed notification
        self.completed.notify_waiters();

        info!("Session {} completed", self.session_id);
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.shutdown.notify_waiters();
        self.completed.notified().await;
    }

    async fn handle_client_packet(self: &Arc<Self>) -> Result<bool> {
        // Read and parse MQTT packet from stream
        let mut stream = self.stream.write().await;
        
        // Parse and validate packet directly from stream
        match Packet::decode(&mut *stream).await {
            Ok(packet) => {
                // Handle different packet types
                match packet {
                    Packet::Connect(p) => self.handle_connect(p).await?,
                    Packet::Publish(p) => self.handle_publish(p).await?,
                    Packet::Subscribe(p) => self.handle_subscribe(p).await?,
                    Packet::Unsubscribe(p) => self.handle_unsubscribe(p).await?,
                    Packet::PingReq => self.handle_pingreq().await?,
                    Packet::Disconnect => {
                        self.handle_disconnect().await?;
                        return Ok(false); // End session
                    }
                    _ => {
                        error!("Unhandled packet type for session {}", self.session_id);
                    }
                }
            }
            Err(e) => {
                // Check if this is a connection closed error
                if let Some(io_error) = e.source().and_then(|e| e.downcast_ref::<std::io::Error>()) {
                    if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                        info!("Client {} closed connection", self.session_id);
                        return Ok(false);
                    }
                }
                error!("Packet decoding error for session {}: {}", self.session_id, e);
                return Err(e.into());
            }
        }

        Ok(true) // Continue session
    }

    async fn handle_connect(self: &Arc<Self>, packet: crate::protocol::packet::ConnectPacket) -> Result<()> {
        info!("CONNECT received for session {}: client_id={}, clean_session={}", self.session_id, packet.client_id, packet.clean_session);

        // Update client_id if provided
        if !packet.client_id.is_empty() {
            let formatted_client_id = format!("__client_{}", packet.client_id);
            *self.client_id.write().await = formatted_client_id;
        }

        // Set connected flag
        *self.connected.write().await = true;

        // Register session with server
        self.server.register_session(Arc::clone(self)).await?;

        // Send CONNACK response
        let connack = crate::protocol::packet::ConnAckPacket {
            session_present: false,
            return_code: crate::protocol::v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = bytes::BytesMut::new();
        Packet::ConnAck(connack).encode(&mut buf);
        self.stream.write().await.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_publish(&self, packet: crate::protocol::packet::PublishPacket) -> Result<()> {
        // Check if connected
        if !*self.connected.read().await {
            error!("PUBLISH received for session {} but not connected", self.session_id);
            return Ok(());
        }

        info!("PUBLISH received for session {}: topic={}, qos={:?}", self.session_id, packet.topic, packet.qos);
        // Here you would route the message through the server's router
        // self.server.route_message(packet).await?;
        Ok(())
    }

    async fn handle_subscribe(&self, packet: crate::protocol::packet::SubscribePacket) -> Result<()> {
        // Check if connected
        if !*self.connected.read().await {
            error!("SUBSCRIBE received for session {} but not connected", self.session_id);
            return Ok(());
        }

        info!("SUBSCRIBE received for session {}: topics={:?}", self.session_id, packet.topic_filters);
        // Handle subscription logic and send SUBACK
        let suback = crate::protocol::packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes: vec![crate::protocol::v3::subscribe_return_codes::MAXIMUM_QOS_0; packet.topic_filters.len()],
        };
        let mut buf = bytes::BytesMut::new();
        Packet::SubAck(suback).encode(&mut buf);
        self.stream.write().await.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_pingreq(&self) -> Result<()> {
        // Check if connected
        if !*self.connected.read().await {
            error!("PINGREQ received for session {} but not connected", self.session_id);
            return Ok(());
        }

        info!("PINGREQ received for session {}", self.session_id);
        let pingresp = Packet::PingResp;
        let mut buf = bytes::BytesMut::new();
        pingresp.encode(&mut buf);
        self.stream.write().await.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_unsubscribe(&self, packet: crate::protocol::packet::UnsubscribePacket) -> Result<()> {
        // Check if connected
        if !*self.connected.read().await {
            error!("UNSUBSCRIBE received for session {} but not connected", self.session_id);
            return Ok(());
        }

        info!("UNSUBSCRIBE received for session {}: topics={:?}", self.session_id, packet.topic_filters);
        // Handle unsubscription logic and send UNSUBACK
        let unsuback = crate::protocol::packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        let mut buf = bytes::BytesMut::new();
        Packet::UnsubAck(unsuback).encode(&mut buf);
        self.stream.write().await.write_all(&buf).await?;
        Ok(())
    }

    async fn handle_disconnect(&self) -> Result<()> {
        info!("DISCONNECT received for session {}", self.session_id);
        // Notify shutdown
        self.shutdown.notify_waiters();
        Ok(())
    }

    async fn handle_message(&self, message: bytes::Bytes) -> Result<()> {
        // Forward the message and encode it as a packet to AsyncWrite of the stream
        info!("Forwarding message to session {}", self.session_id);
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
