use crate::{
    broker::Broker,
    protocol::packet, protocol::v3,
    transport::AsyncStream,
};
use std::{pin::Pin, future::Future, sync::Arc, collections::VecDeque};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
    time::{Duration, Instant},
};
use tracing::{debug, info, error};
use uuid::Uuid;
use thiserror::Error;
use bytes::{Bytes, BytesMut};

pub struct Session {
    id: String,
    command_tx: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

struct Runtime {
    id: String,
    client_id: Option<String>,
    broker: Arc<Broker>,
    takeover: TakeoverAction,
    command_rx: mpsc::Receiver<Command>,
    message_tx: mpsc::Sender<packet::Packet>,
    message_rx: mpsc::Receiver<packet::Packet>,
    stream: Option<Box<dyn AsyncStream>>,
    clean_session: bool,
    keep_alive: u16,
    will_message: Option<WillMessage>,
    next_packet_id: u16,  // For generating packet IDs for outgoing QoS > 0 messages
    qos1_queue: VecDeque<packet::PublishPacket>,  // Queue for QoS=1 messages waiting to be sent
    qos1_pending: Option<InflightMessage>,  // Currently in-flight QoS=1 message
}

#[derive(Debug, Clone)]
struct InflightMessage {
    packet: packet::PublishPacket,
    timestamp: Instant,
    retry_count: u32,
    next_retry_interval_ms: u64,
}

pub type Mailbox = mpsc::Sender<packet::Packet>;

#[derive(Debug, Clone)]
struct WillMessage {
    topic: String,
    payload: Bytes,
    _qos: packet::QoS,
    retain: bool,
}

enum Command {
    Takeover(Box<dyn AsyncStream>, bool, u16),
    Cancel,
}

#[derive(Clone, Copy)]
enum State {
    WaitConnect,
    Processing,
    WaitTakeover,
    Cleanup,
}

#[derive(Error, Debug)]
enum SessionError {
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Send channel error: {0}")]
    SendError(#[from] mpsc::error::SendError<packet::Packet>),
}

type Result<T> = std::result::Result<T, SessionError>;

pub type TakeoverAction = Arc<dyn Fn(Box<dyn AsyncStream>, bool, u16) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync + 'static>;

impl Session {
    pub async fn spawn(broker: Arc<Broker>, stream: Box<dyn AsyncStream>) -> Session {
        let id = Uuid::new_v4().to_string();
        let (command_tx, command_rx) = mpsc::channel(10);
        let (message_tx, message_rx) = mpsc::channel(100);

        Self {
            id: id.clone(),
            command_tx: command_tx.clone(),

            task: tokio::spawn(async move {
                let mut runtime = Runtime {
                    id,
                    client_id: None,
                    broker,
                    takeover: Self::make_takeover(command_tx),
                    command_rx,
                    message_tx,
                    message_rx,
                    stream: Some(stream),
                    clean_session: true,
                    keep_alive: 0,
                    will_message: None,
                    next_packet_id: 1,  // Start from 1, 0 is reserved
                    qos1_queue: VecDeque::new(),
                    qos1_pending: None,
                };

                runtime.run().await;
                runtime.broker.remove_session(&runtime.id, runtime.client_id.as_ref()).await;
            }),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn make_takeover(command_tx: mpsc::Sender<Command>) -> TakeoverAction {
        Arc::new(move |stream: Box<dyn AsyncStream>, clean_session: bool, keep_alive: u16| {
            let command_tx2 = command_tx.clone();

            Box::pin(async move {
                if let Err(e) = command_tx2.send(
                    Command::Takeover(
                        stream, clean_session, keep_alive)).await {
                    error!("Failed to send takeover message: {}", e);
                }
            })
        })
    }

    pub async fn cancel(self) -> JoinHandle<()> {
        self.command_tx.send(Command::Cancel).await.unwrap_or_else(|e| {
            info!("Session canceled too early: {}", e);
        });
        self.task
    }
}

impl Runtime {
    fn next_packet_id(&mut self) -> u16 {
        let id = self.next_packet_id;
        self.next_packet_id = if self.next_packet_id == 65535 {
            1  // Wrap around, skipping 0
        } else {
            self.next_packet_id + 1
        };
        id
    }

    /// Calculate the next retry interval using exponential backoff
    fn calculate_next_retry_interval(
        base_interval_ms: u64,
        retry_count: u32,
        multiplier: f32,
        max_interval_ms: u64
    ) -> u64 {
        if base_interval_ms == 0 {
            return 0; // Retransmission disabled
        }

        // Ensure multiplier is at least 1.0
        let safe_multiplier = multiplier.max(1.0);

        // Calculate exponential interval
        let exponential_interval = (base_interval_ms as f64) * 
            (safe_multiplier as f64).powi(retry_count as i32);

        // Cap at maximum interval
        let capped_interval = exponential_interval.min(max_interval_ms as f64);

        capped_interval as u64
    }

    async fn run(&mut self) {
        let mut state = State::WaitConnect;
        loop {
            state = match state {
                State::WaitConnect => self.do_wait_connect().await,
                State::Processing => self.do_processing().await,
                State::WaitTakeover => self.do_wait_takeover().await,
                State::Cleanup => {
                    debug!("Session {} STATE CLEANUP", self.id);
                    // TODO: Save subscriptions and unfinished messages to persistent storage
                    // This will be implemented in Milestone 3 for persistent session support
                    break;
                },
            }
        }
        debug!("Session {} EXIT RUN", self.id);
    }

    async fn do_wait_connect(&mut self) -> State {
        debug!("Session {} STATE WAIT_CONNECT", self.id);

        let mut stream = match self.stream.take() {
            Some(stream) => stream,
            None => { return State::Cleanup; },
        };

        tokio::select! {
            pack = packet::Packet::decode(&mut stream) => {
                match pack {
                    Ok(packet::Packet::Connect(connect)) => {
                        match self.on_connect(connect, stream).await {
                            Ok(state) => state,
                            Err(e) => {
                                info!("Session IO error: {}", e);
                                State::Cleanup
                            },
                        }
                    },
                    Ok(_) => State::Cleanup,
                    Err(e) => {
                        info!("Session IO error: {}", e);
                        State::Cleanup
                    }
                }
            }

            _ = self.command_rx.recv() => State::Cleanup
        }
    }

    async fn do_processing(&mut self) -> State {
        debug!("Session {} STATE PROCESSING", self.id);

        let mut stream = match self.stream.take() {
            Some(stream) => stream,
            None => {
                error!("Error processing session {} with no client stream", self.id);
                return State::Cleanup;
            }
        };
        let (mut reader, mut writer) = stream.split();
        let mut next_state = State::Cleanup;

        // Keep-alive setup
        let keep_alive_secs = self.keep_alive;
        let keep_alive_timeout = if keep_alive_secs > 0 {
            // MQTT spec: disconnect if no packet received within 1.5x keep-alive interval
            Duration::from_millis(keep_alive_secs as u64 * 1500)
        } else {
            Duration::from_secs(3600 * 24 * 365) // Effectively infinite (1 year)
        };

        let mut keep_alive_interval = tokio::time::interval(keep_alive_timeout);
        keep_alive_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the first tick since interval fires immediately on creation
        keep_alive_interval.tick().await;

        // Retransmission timer for QoS=1 messages
        let retransmission_interval_ms = self.broker.config().server.get_retransmission_interval_ms();
        let retransmission_enabled = retransmission_interval_ms > 0;

        // Timer ticks at half the retransmission interval to check which messages are due
        let timer_interval = if retransmission_enabled {
            // Tick at half the interval for timely checks
            Duration::from_millis(retransmission_interval_ms / 2)
        } else {
            Duration::from_secs(3600 * 24) // 24 hours - effectively disabled
        };

        let mut retransmit_interval = tokio::time::interval(timer_interval);
        retransmit_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the first tick
        retransmit_interval.tick().await;

        loop {
            tokio::select! {
                pack = packet::Packet::decode(&mut reader) => {
                    keep_alive_interval.reset(); // Reset the interval on any packet
                    let success = match pack {
                        Ok(pack) => {
                            debug!("Session {} received packet: {:?}", self.id, pack);
                            self.on_packet(pack, &mut writer).await.unwrap_or_else(|e| {
                                info!("Session {} error: {}", self.id, e);
                                false
                            })
                        },
                        Err(e) => {
                            info!("Session {} error: {}", self.id, e);
                            false
                        }
                    };
                    if !success {
                        if !self.clean_session {
                            next_state = State::WaitTakeover;
                        }
                        break;
                    }
                }

                Some(message) = self.message_rx.recv() => {
                    if let Err(e) = self.on_message(message, &mut writer).await {
                        info!("Session {} error: {}", self.id, e);
                        if !self.clean_session {
                            next_state = State::WaitTakeover;
                        }
                        break;
                    }
                }

                command = self.command_rx.recv() => {
                    if let Some(Command::Takeover(
                            mut new_stream, clean_session, keep_alive)) = command {
                        // Disconnect original client
                        let mut buf = BytesMut::new();
                        packet::Packet::Disconnect.encode(&mut buf);
                        let _ = writer.write_all(&buf).await;

                        // Process takeover message
                        match self.on_takeover(&mut new_stream, clean_session, keep_alive).await {
                            Ok(_) => {
                                self.stream.replace(new_stream);
                                next_state = State::Processing;
                            },
                            Err(e) => {
                                info!("Session {} error: {}", self.id, e);
                                if !clean_session {
                                    next_state = State::WaitTakeover;
                                }
                            }
                        }
                    }

                    // Always break for any reason
                    break;
                }

                _ = keep_alive_interval.tick() => {
                    if keep_alive_secs > 0 {
                        info!("Keep-alive timeout for session {} ({}s without activity)", 
                            self.id, keep_alive_secs);
                        if !self.clean_session {
                            next_state = State::WaitTakeover;
                        }
                        // Keep-alive timeout is an abnormal disconnect
                        break;
                    }
                    // If keep_alive_secs == 0, this tick is ignored (infinite timeout)
                }

                _ = retransmit_interval.tick() => {
                    if let Err(e) = self.on_retransmit(&mut writer).await {
                        error!("Retransmission error: {}", e);
                        if !self.clean_session {
                            next_state = State::WaitTakeover;
                        }
                        break;
                    }
                }
            }
        }

        // Publish Will message if we're exiting abnormally (going to Cleanup)
        // but NOT if we're going to WaitTakeover (persistent session)
        if matches!(next_state, State::Cleanup) {
            self.publish_will().await;
        }

        next_state
    }

    async fn do_wait_takeover(&mut self) -> State {
        debug!("Session {} STATE WAIT_TAKEOVER", self.id);

        match self.command_rx.recv().await {
            Some(Command::Takeover(mut stream, clean_session, keep_alive)) => {
                match self.on_takeover(&mut stream, clean_session, keep_alive).await {
                    Ok(_) => {
                        self.stream.replace(stream);
                        State::Processing
                    },
                    Err(e) => {
                        info!("Session {} error: {}", self.id, e);
                        State::WaitTakeover
                    }
                }
            },
            Some(Command::Cancel) => State::Cleanup,
            _ => State::Cleanup,
        }
    }

    async fn on_connect(&mut self, connect: packet::ConnectPacket, mut stream: Box<dyn AsyncStream>) -> Result<State> {
        info!("CONNECT received: client_id={}, clean_session={}, will_flag={}", 
            connect.client_id, connect.clean_session, connect.will_flag);

        // Validate protocol name and version
        if connect.protocol_name != v3::PROTOCOL_NAME || connect.protocol_level != v3::PROTOCOL_LEVEL {
            info!("Invalid protocol: name={}, level={}", connect.protocol_name, connect.protocol_level);
            let connack = packet::ConnAckPacket {
                session_present: false,
                return_code: v3::connect_return_codes::UNACCEPTABLE_PROTOCOL_VERSION,
            };
            let mut buf = BytesMut::new();
            packet::Packet::ConnAck(connack).encode(&mut buf);
            stream.write_all(&buf).await?;
            return Ok(State::Cleanup);
        }

        // Validate client ID
        if !connect.client_id.is_empty() {
            // Check length (23 UTF-8 bytes max)
            if connect.client_id.len() > 23 {
                info!("Client ID too long: {} bytes", connect.client_id.len());
                let connack = packet::ConnAckPacket {
                    session_present: false,
                    return_code: v3::connect_return_codes::IDENTIFIER_REJECTED,
                };
                let mut buf = BytesMut::new();
                packet::Packet::ConnAck(connack).encode(&mut buf);
                stream.write_all(&buf).await?;
                return Ok(State::Cleanup);
            }

            // Check character set (0-9, a-z, A-Z)
            if !connect.client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
                info!("Client ID contains invalid characters: {}", connect.client_id);
                let connack = packet::ConnAckPacket {
                    session_present: false,
                    return_code: v3::connect_return_codes::IDENTIFIER_REJECTED,
                };
                let mut buf = BytesMut::new();
                packet::Packet::ConnAck(connack).encode(&mut buf);
                stream.write_all(&buf).await?;
                return Ok(State::Cleanup);
            }
        } else if !connect.clean_session {
            // Empty client ID with clean_session=false is not allowed
            let connack = packet::ConnAckPacket {
                session_present: false,
                return_code: v3::connect_return_codes::IDENTIFIER_REJECTED,
            };
            let mut buf = BytesMut::new();
            packet::Packet::ConnAck(connack).encode(&mut buf);
            stream.write_all(&buf).await?;
            return Ok(State::Cleanup);
        }

        // Update session parameters
        self.clean_session= connect.clean_session;
        self.keep_alive = connect.keep_alive;

        // Store Will message if present
        if connect.will_flag {
            if let (Some(topic), Some(payload)) = (connect.will_topic, connect.will_payload) {
                let will = WillMessage {
                    topic,
                    payload,
                    _qos: connect.will_qos,
                    retain: connect.will_retain,
                };
                self.will_message.replace(will);
                info!("Stored Will message for session {}", self.id);
            }
        }

        // Handle client ID and potential session takeover
        if !connect.client_id.is_empty() {
            self.client_id.replace(connect.client_id.clone());

            // Check client ID collision and do take over
            if let Some(takeover) = self.broker.has_collision(&connect.client_id, self.takeover.clone()).await {
                info!("Client {} already connected, initiating takeover", connect.client_id);

                takeover(stream, connect.clean_session, connect.keep_alive).await;
                return Ok(State::Cleanup); // Exit this session task
            }
        }

        // Send CONNACK response
        let connack = packet::ConnAckPacket {
            session_present: false, // New connection, no existing session
            return_code: crate::protocol::v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        stream.write_all(&buf).await?;

        self.stream.replace(stream);
        Ok(State::Processing)
    }

    async fn on_takeover(&mut self, stream: &mut Box<dyn AsyncStream>, clean_session: bool, keep_alive: u16) -> Result<()> {
        // Session present is true if clean_session=false (persistent session was resumed)
        let session_present = !clean_session;
        let connack = packet::ConnAckPacket {
            session_present,
            return_code: v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        match stream.write_all(&buf).await {
            Ok(_) => {
                self.clean_session = clean_session;
                self.keep_alive = keep_alive;
                if clean_session {
                    self.broker.unsubscribe_all(&self.id).await;
                }
                Ok(())
            },
            Err(e) => {
                Err(e.into())
            }
        }
    }

    async fn on_packet<W: AsyncWrite + Unpin>(&mut self, pack: packet::Packet, writer: &mut W) -> Result<bool> {
        // Read and parse MQTT packet from stream
        match pack {
            packet::Packet::Publish(p) => self.on_publish(p, writer).await,
            packet::Packet::PubAck(p) => self.on_puback(p, writer).await,
            packet::Packet::Subscribe(p) => self.on_subscribe(p, writer).await,
            packet::Packet::Unsubscribe(p) => self.on_unsubscribe(p, writer).await,
            packet::Packet::PingReq => self.on_pingreq(writer).await,
            packet::Packet::Disconnect => self.on_disconnect().await,
            _ => {
                error!("Unhandled packet type for session {}", self.id);
                Ok(false)
            }
        }
    }

    async fn send_next_qos1_message<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<()> {
        if self.qos1_pending.is_some() {
            return Ok(());
        }

        if let Some(mut packet) = self.qos1_queue.pop_front() {
            // Assign packet ID
            let packet_id = self.next_packet_id();
            packet.packet_id = Some(packet_id);

            debug!("Sending QoS=1 message from queue with packet_id={}, remaining: {}", 
                packet_id, self.qos1_queue.len());

            // Track for retransmission
            let config = self.broker.config();
            let base_interval = config.server.get_retransmission_interval_ms();
            let initial_retry_interval = Self::calculate_next_retry_interval(
                base_interval,
                0,
                config.server.get_backoff_multiplier(),
                config.server.retransmission_max_interval_ms
            );

            let inflight = InflightMessage {
                packet: packet.clone(),
                timestamp: Instant::now(),
                retry_count: 0,
                next_retry_interval_ms: initial_retry_interval,
            };

            // Mark as pending
            self.qos1_pending = Some(inflight);

            // Send the packet
            let mut buf = BytesMut::new();
            packet::Packet::Publish(packet).encode(&mut buf);
            writer.write_all(&buf).await?;
        }
        Ok(())
    }

    async fn on_message<W: AsyncWrite + Unpin>(&mut self, message: packet::Packet, writer: &mut W) -> Result<()> {
        match message {
            // Handle PUBLISH packets
            packet::Packet::Publish(pub_packet) => {
                // Check if this is a new message without packet_id (from router)
                if pub_packet.qos == packet::QoS::AtLeastOnce {
                    // Event 1: New QoS=1 message - queue it
                    debug!("Received new QoS=1 message for ordered delivery");
                    self.qos1_queue.push_back(pub_packet);

                    // If no message is pending, send the next one
                    self.send_next_qos1_message(writer).await?;
                } else {
                    // QoS=0 or already has packet_id (retransmission)
                    let mut buf = BytesMut::new();
                    packet::Packet::Publish(pub_packet).encode(&mut buf);
                    writer.write_all(&buf).await?;
                }
            }

            // Ignore other packet types (PUBACK, SUBACK, PINGRESP, etc) to client
            _ => {
            }
        }
        Ok(())
    }

    async fn on_puback<W: AsyncWrite + Unpin>(&mut self, packet: packet::PubAckPacket, writer: &mut W) -> Result<bool> {
        info!("PUBACK received for session {}: packet_id={}", self.id, packet.packet_id);

        // Event 3: Send PUBACK to mailbox
        // Check if this acknowledges the pending message
        if let Some(pending) = &self.qos1_pending {
            if pending.packet.packet_id == Some(packet.packet_id) {
                debug!("PUBACK acknowledged pending QoS=1 message");

                // Clear pending and send next message
                self.qos1_pending = None;
                self.send_next_qos1_message(writer).await?;
            }
        }

        Ok(true)
    }

    async fn on_publish<W: AsyncWrite + Unpin>(&mut self, packet: packet::PublishPacket, writer: &mut W) -> Result<bool> {
        info!("PUBLISH received for session {}: topic={}, qos={:?}, retain={}, dup={}", 
            self.id, packet.topic, packet.qos, packet.retain, packet.dup);

        // Handle QoS=1: Send PUBACK if required
        if packet.qos == packet::QoS::AtLeastOnce {
            if let Some(packet_id) = packet.packet_id {
                // Always send PUBACK for QoS=1 (even for duplicates)
                let puback = packet::PubAckPacket { packet_id };
                let mut buf = BytesMut::new();
                packet::Packet::PubAck(puback).encode(&mut buf);
                writer.write_all(&buf).await?;
                info!("Sent PUBACK for packet_id={} to session {}", packet_id, self.id);

                // Check DUP flag - if it's a duplicate, don't route
                if packet.dup {
                    info!("Received duplicate PUBLISH with packet_id={}, not routing", packet_id);
                    return Ok(true); // Still return success, just don't route
                }
            } else {
                // QoS=1 requires packet_id
                error!("QoS=1 PUBLISH missing packet_id from session {}", self.id);
                return Ok(false); // Disconnect on protocol violation
            }
        }

        // Route the message through the router (only for non-duplicates or QoS=0)
        self.broker.route(packet).await;

        Ok(true)
    }

    async fn on_subscribe<W: AsyncWrite + Unpin>(&mut self, packet: packet::SubscribePacket, writer: &mut W) -> Result<bool> {
        info!("SUBSCRIBE received for session {}: topics={:?}", self.id, packet.topic_filters);

        // Handle subscription logic through router
        let (return_codes, retained_messages) = self.broker.subscribe(
            &self.id,
            self.message_tx.clone(),
            &packet.topic_filters,
        ).await;

        // Send SUBACK first
        let suback = packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes,
        };
        let mut buf = BytesMut::new();
        packet::Packet::SubAck(suback).encode(&mut buf);
        writer.write_all(&buf).await?;

        // Then send any retained messages
        for retained_msg in retained_messages {
            self.message_tx.send(packet::Packet::Publish(retained_msg)).await?;
        }

        Ok(true)
    }

    async fn on_unsubscribe<W: AsyncWrite + Unpin>(&mut self, packet: packet::UnsubscribePacket, writer: &mut W) -> Result<bool> {
        info!("UNSUBSCRIBE received for session {}: topics={:?}", self.id, packet.topic_filters);

        // Handle unsubscription logic through router
        self.broker.unsubscribe(&self.id, &packet.topic_filters).await;

        // Send UNSUBACK
        let unsuback = packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        let mut buf = BytesMut::new();
        packet::Packet::UnsubAck(unsuback).encode(&mut buf);
        writer.write_all(&buf).await?;
        Ok(true)
    }

    async fn on_pingreq<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<bool> {
        info!("PINGREQ received for session {}", self.id);
        let mut buf = BytesMut::new();
        packet::Packet::PingResp.encode(&mut buf);
        writer.write_all(&buf).await?;
        Ok(true)
    }

    async fn on_disconnect(&mut self) -> Result<bool> {
        info!("DISCONNECT received for session {}", self.id);
        // Clear Will message on normal disconnect
        self.will_message.take();
        Ok(false)
    }

    async fn on_retransmit<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<()> {
        // Event 2: Retransmission tick
        // Get config values before any mutable borrows
        let retransmission_interval_ms = self.broker.config().server.get_retransmission_interval_ms();
        if retransmission_interval_ms == 0 {
            return Ok(()); // Continue - retransmission disabled
        }

        let max_retransmission_limit = self.broker.config().server.max_retransmission_limit;
        let backoff_multiplier = self.broker.config().server.get_backoff_multiplier();
        let retransmission_max_interval_ms = self.broker.config().server.retransmission_max_interval_ms;

        if let Some(inflight) = &self.qos1_pending {
            let now = Instant::now();
            let elapsed_ms = now.duration_since(inflight.timestamp).as_millis() as u64;

            // Check if interval is ok for retry
            if elapsed_ms >= inflight.next_retry_interval_ms {
                // Check max retransmission limit first
                if inflight.retry_count >= max_retransmission_limit {
                    info!("Message packet_id={:?} exceeded max retransmission limit ({}), dropping", 
                        inflight.packet.packet_id, max_retransmission_limit);

                    // Clear pending
                    self.qos1_pending = None;

                    // After dropping, send next message from queue
                    self.send_next_qos1_message(writer).await?;
                } else {
                    // Send the packet with DUP flag
                    let mut packet = inflight.packet.clone();
                    packet.dup = true;

                    debug!("Retransmitting QoS=1 message with packet_id={:?} for session {}", 
                        packet.packet_id, self.id);

                    let mut buf = BytesMut::new();
                    packet::Packet::Publish(packet).encode(&mut buf);
                    writer.write_all(&buf).await?;

                    // Update inflight tracking
                    if let Some(inflight) = &mut self.qos1_pending {
                        inflight.timestamp = now;
                        inflight.retry_count += 1;
                        inflight.next_retry_interval_ms = Self::calculate_next_retry_interval(
                            retransmission_interval_ms,
                            inflight.retry_count,
                            backoff_multiplier,
                            retransmission_max_interval_ms
                        );

                        info!("Retransmitted message packet_id={:?}, retry_count={}/{}, next retry in {}ms", 
                            inflight.packet.packet_id, inflight.retry_count, max_retransmission_limit, 
                            inflight.next_retry_interval_ms);
                    }
                }
            }
            // else: interval not ok, just return (continue)
        }
        // else: No pending message - this is normal

        Ok(()) // Continue
    }

    async fn publish_will(&mut self) {
        if let Some(will) = self.will_message.take() {
            info!("Publishing Will message for session {}: topic={}", self.id, will.topic);
            let packet = packet::PublishPacket {
                topic: will.topic,
                packet_id: None, // Will messages are QoS 0 for Milestone 1
                payload: will.payload,
                qos: packet::QoS::AtMostOnce, // For Milestone 1, treat all as QoS 0
                retain: will.retain,
                dup: false,
            };
            self.broker.route(packet).await;
        }
    }
}
