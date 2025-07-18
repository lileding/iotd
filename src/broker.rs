use crate::session::Session;
use crate::router::Router;
use crate::protocol::packet;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use std::error::Error;
use std::sync::atomic::Ordering;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tracing::{info, error};

pub struct Broker {
    sessions: Mutex<HashMap<String, JoinHandle<()>>>,
    named_clients: DashMap<String, (String, CancellationToken)>,
    #[allow(dead_code)]
    router: Router,
    shutdown: CancellationToken,
}

unsafe impl Send for Broker {}
unsafe impl Sync for Broker {}

impl Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            named_clients: DashMap::new(),
            router: Router::new(),
            shutdown: CancellationToken::new(),
        })
    }

    pub async fn add_session(self: &Arc<Self>, session: Session) {
        let session_id = session.session_id.clone();
        info!("Added session {} to broker", session_id);
        
        // Spawn a thread to handle client packets
        let broker_run = Arc::clone(self);
        let shutdown = self.shutdown.child_token();
        let handle = tokio::spawn(async move {
            Broker::process_session(broker_run, session, shutdown).await;
        });
        
        self.sessions.lock().await.insert(session_id, handle);
    }

    pub async fn clean_all_sessions(&self) {
        // Lock the sessions to avoid another add_session call
        let mut sessions = self.sessions.lock().await;
        
        // Trigger shutdown signal
        self.shutdown.cancel();
        
        // Wait for every handle to complete in sessions
        for (session_id, handle) in sessions.drain() {
            if let Err(e) = handle.await {
                error!("Error waiting for session {} to complete: {}", session_id, e);
            }
        }
        
        // Clean sessions and named_clients
        sessions.clear();
        self.named_clients.clear();

        info!("All sessions cleaned");
    }

    pub async fn process_session(self: Arc<Self>, mut session: Session, shutdown: CancellationToken) {
        info!("Starting session processing for session {}", session.session_id);
        
        loop {
            tokio::select! {
                result = self.process_packet(&mut session, shutdown.clone()) => {
                    match result {
                        Ok(true) => {
                            // Continue processing packets
                        },
                        Ok(false) => {
                            info!("Session {} ended normally", session.session_id);
                            break;
                        },
                        Err(e) => {
                            error!("Error processing packet for session {}: {}", session.session_id, e);
                            break;
                        },
                    }
                }

                _ = shutdown.cancelled() => {
                    info!("Session {} shutting down due to broker shutdown", session.session_id);
                    
                    // Send DISCONNECT message if session is connected
                    if session.connected.load(Ordering::Relaxed) {
                        let disconnect = packet::Packet::Disconnect;
                        let mut buf = bytes::BytesMut::new();
                        disconnect.encode(&mut buf);
                        if let Err(e) = session.stream.write_all(&buf).await {
                            error!("Failed to send disconnect message for session {}: {}", session.session_id, e);
                        }
                    }
                    
                    break;
                },
            }
        }
        
        // Remove session from sessions map
        self.sessions.lock().await.remove(&session.session_id);
        
        info!("Session {} processing completed", session.session_id);
    }

    async fn process_packet(self: &Arc<Self>, session: &mut Session, shutdown: CancellationToken) -> Result<bool> {
        // Read and parse MQTT packet from stream
        match packet::Packet::decode(&mut session.stream).await {
            Ok(packet) => {
                // Handle different packet types
                return match packet {
                    packet::Packet::Connect(p) => self.on_connect(session, p, shutdown).await,
                    packet::Packet::Publish(p) => self.on_publish(session, p).await,
                    packet::Packet::Subscribe(p) => self.on_subscribe(session, p).await,
                    packet::Packet::Unsubscribe(p) => self.on_unsubscribe(session, p).await,
                    packet::Packet::PingReq => self.on_pingreq(session).await,
                    packet::Packet::Disconnect => self.on_disconnect(session).await,
                    _ => {
                        let session_id = &session.session_id;
                        error!("Unhandled packet type for session {}", session_id);
                        Ok(false)
                    }
                }
            },
            Err(e) => {
                // Check if this is a connection closed error
                if let Some(io_error) = e.source().and_then(|e| e.downcast_ref::<std::io::Error>()) {
                    if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                        let session_id = &session.session_id;
                        info!("Client {} closed connection", session_id);
                        return Ok(false);
                    }
                }
                let session_id = &session.session_id;
                error!("Packet decoding error for session {}: {}", session_id, e);
                Err(e.into())
            }
        }
    }

    async fn on_connect(self: &Arc<Self>, session: &mut Session, packet: packet::ConnectPacket, shutdown: CancellationToken) -> Result<bool> {
        info!("CONNECT received: client_id={}, clean_session={}", packet.client_id, packet.clean_session);

        // Set connected flag to true before sending CONNACK
        session.connected.store(true, Ordering::Relaxed);

        // Send CONNACK response
        let connack = packet::ConnAckPacket {
            session_present: false,
            return_code: crate::protocol::v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        session.stream.write_all(&buf).await?;

        // Update client ID if provided and handle name collision after CONNACK
        if !packet.client_id.is_empty() {
            session.client_id = Some(packet.client_id.clone());
            // Remove name collisioned session
            if let Some((old_session_id, cancel)) = self.named_clients.insert(packet.client_id.clone(), (session.session_id.clone(), shutdown)) {
                info!("Client {} already connected on session {}, cancelling old session", packet.client_id, old_session_id);
                cancel.cancel();
                if let Some(handle) = self.sessions.lock().await.remove(&old_session_id) {
                    if let Err(e) = handle.await {
                        error!("Error waiting for session {} to complete: {}", old_session_id, e);
                    }
                }
            }
        }

        Ok(true)
    }

    async fn on_publish(self: &Arc<Self>, session: &mut Session, packet: packet::PublishPacket) -> Result<bool> {
        let session_id = &session.session_id;
        info!("PUBLISH received for session {}: topic={}, qos={:?}", session_id, packet.topic, packet.qos);
        // Route the message through the router
        // self.router.route_message(packet).await?;
        Ok(true)
    }

    async fn on_subscribe(self: &Arc<Self>, session: &mut Session, packet: packet::SubscribePacket) -> Result<bool> {
        let session_id = &session.session_id;
        info!("SUBSCRIBE received for session {}: topics={:?}", session_id, packet.topic_filters);
        
        // Handle subscription logic and send SUBACK
        let suback = packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes: vec![crate::protocol::v3::subscribe_return_codes::MAXIMUM_QOS_0; packet.topic_filters.len()],
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::SubAck(suback).encode(&mut buf);
        session.stream.write_all(&buf).await?;
        Ok(true)
    }

    async fn on_pingreq(self: &Arc<Self>, session: &mut Session) -> Result<bool> {
        let session_id = &session.session_id;
        info!("PINGREQ received for session {}", session_id);
        let pingresp = packet::Packet::PingResp;
        let mut buf = bytes::BytesMut::new();
        pingresp.encode(&mut buf);
        session.stream.write_all(&buf).await?;
        Ok(true)
    }

    async fn on_unsubscribe(self: &Arc<Self>, session: &mut Session, packet: packet::UnsubscribePacket) -> Result<bool> {
        let session_id = &session.session_id;
        info!("UNSUBSCRIBE received for session {}: topics={:?}", session_id, packet.topic_filters);
        
        // Handle unsubscription logic and send UNSUBACK
        let unsuback = packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::UnsubAck(unsuback).encode(&mut buf);
        session.stream.write_all(&buf).await?;
        Ok(true)
    }

    async fn on_disconnect(self: &Arc<Self>, session: &mut Session) -> Result<bool> {
        let session_id = &session.session_id;
        info!("DISCONNECT received for session {}", session_id);
        Ok(false)
    }
}
