use crate::storage::{
    PersistedInboundQos2Message, PersistedInflightMessage, PersistedQos2State, PersistedSession,
    PersistedSubscription, PersistedWillMessage, StoredQoS,
};
use crate::{broker::Broker, protocol::packet, protocol::v3, transport::AsyncStream};
use bytes::{Bytes, BytesMut};
use chrono::Utc;
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
    time::{Duration, Instant},
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

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
    next_packet_id: u16, // For generating packet IDs for outgoing QoS > 0 messages
    inflight_queue: VecDeque<InflightMessage>, // In-flight QoS=1 and QoS=2 messages
    inbound_qos2: HashMap<u16, ReceivedQos2Message>, // Inbound QoS=2 messages awaiting PUBREL
}

/// QoS=2 message state for outbound messages
#[derive(Debug, Clone, Copy, PartialEq)]
enum Qos2State {
    AwaitingPubRec, // PUBLISH sent, waiting for PUBREC
    AwaitingPubComp, // PUBREL sent, waiting for PUBCOMP
}

#[derive(Debug, Clone)]
struct InflightMessage {
    packet: packet::PublishPacket,
    timestamp: Instant,
    retry_count: u32,
    qos2_state: Option<Qos2State>, // None for QoS=1, Some for QoS=2
}

/// Inbound QoS=2 message waiting for PUBREL
#[derive(Debug, Clone)]
struct ReceivedQos2Message {
    packet: packet::PublishPacket,
}

pub type Mailbox = mpsc::Sender<packet::Packet>;

#[derive(Debug, Clone)]
struct WillMessage {
    topic: String,
    payload: Bytes,
    qos: packet::QoS,
    retain: bool,
}

struct TakeoverParams {
    stream: Box<dyn AsyncStream>,
    clean_session: bool,
    keep_alive: u16,
}

enum Command {
    Takeover(TakeoverParams),
    Cancel,
}

#[derive(Clone, Copy)]
enum State {
    Waiting,
    Processing,
    Pending,
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

/// Helper to encode and send a packet
async fn send_packet<W: AsyncWrite + Unpin>(packet: packet::Packet, writer: &mut W) -> Result<()> {
    let mut buf = BytesMut::new();
    packet.encode(&mut buf);
    writer.write_all(&buf).await?;
    Ok(())
}

pub type TakeoverAction = Arc<
    dyn Fn(Box<dyn AsyncStream>, bool, u16) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

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
                    next_packet_id: 1, // Start from 1, 0 is reserved
                    inflight_queue: VecDeque::new(),
                    inbound_qos2: HashMap::new(),
                };

                runtime.run().await;
                runtime
                    .broker
                    .remove_session(&runtime.id, runtime.client_id.as_ref())
                    .await;
            }),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn make_takeover(command_tx: mpsc::Sender<Command>) -> TakeoverAction {
        Arc::new(
            move |stream: Box<dyn AsyncStream>, clean_session: bool, keep_alive: u16| {
                let command_tx2 = command_tx.clone();

                Box::pin(async move {
                    let params = TakeoverParams {
                        stream,
                        clean_session,
                        keep_alive,
                    };
                    if let Err(e) = command_tx2.send(Command::Takeover(params)).await {
                        error!("Failed to send takeover message: {}", e);
                    }
                })
            },
        )
    }

    pub async fn cancel(self) -> JoinHandle<()> {
        self.command_tx
            .send(Command::Cancel)
            .await
            .unwrap_or_else(|e| {
                info!("Session canceled too early: {}", e);
            });
        self.task
    }
}

impl Runtime {
    fn next_packet_id(&mut self) -> u16 {
        let id = self.next_packet_id;
        self.next_packet_id = if self.next_packet_id == 65535 {
            1 // Wrap around, skipping 0
        } else {
            self.next_packet_id + 1
        };
        id
    }

    async fn run(&mut self) {
        let mut state = State::Waiting;
        loop {
            state = match state {
                State::Waiting => self.do_waiting().await,
                State::Processing => self.do_processing().await,
                State::Pending => self.do_pending().await,
                State::Cleanup => {
                    debug!("Session {} STATE CLEANUP", self.id);
                    self.publish_will().await;
                    break;
                }
            }
        }
        debug!("Session {} EXIT RUN", self.id);
    }

    async fn do_waiting(&mut self) -> State {
        debug!("Session {} STATE WAITING", self.id);

        let mut stream = match self.stream.take() {
            Some(stream) => stream,
            None => {
                return State::Cleanup;
            }
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

        let stream = match self.stream.take() {
            Some(stream) => stream,
            None => {
                error!("Error processing session {} with no client stream", self.id);
                return State::Cleanup;
            }
        };
        let (mut reader, mut writer) = stream.into_split();
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
        let retransmission_interval_ms = self.broker.config().get_retransmission_interval_ms();
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
                            next_state = State::Pending;
                        }
                        break;
                    }
                }

                Some(message) = self.message_rx.recv() => {
                    if let Err(e) = self.on_message(message, &mut writer).await {
                        info!("Session {} error: {}", self.id, e);
                        if !self.clean_session {
                            next_state = State::Pending;
                        }
                        break;
                    }
                }

                command = self.command_rx.recv() => {
                    if let Some(Command::Takeover(mut params)) = command {
                        // Disconnect original client
                        let _ = send_packet(packet::Packet::Disconnect, &mut writer).await;

                        // Process takeover message
                        match self.on_takeover(&mut params.stream, params.clean_session, params.keep_alive).await {
                            Ok(_) => {
                                self.stream.replace(params.stream);
                                next_state = State::Processing;
                            },
                            Err(e) => {
                                info!("Session {} error: {}", self.id, e);
                                if !params.clean_session {
                                    next_state = State::Pending;
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
                            next_state = State::Pending;
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
                            next_state = State::Pending;
                        }
                        break;
                    }
                }
            }
        }

        next_state
    }

    async fn do_pending(&mut self) -> State {
        debug!("Session {} STATE PENDING", self.id);

        // Save session state to storage at the beginning of Pending state
        self.save_to_storage().await;

        match self.command_rx.recv().await {
            Some(Command::Takeover(mut params)) => {
                match self
                    .on_takeover(&mut params.stream, params.clean_session, params.keep_alive)
                    .await
                {
                    Ok(_) => {
                        self.stream.replace(params.stream);
                        State::Processing
                    }
                    Err(e) => {
                        info!("Session {} error: {}", self.id, e);
                        State::Pending
                    }
                }
            }
            Some(Command::Cancel) => State::Cleanup,
            _ => State::Cleanup,
        }
    }

    async fn on_connect(
        &mut self,
        connect: packet::ConnectPacket,
        mut stream: Box<dyn AsyncStream>,
    ) -> Result<State> {
        info!(
            "CONNECT received: client_id={}, clean_session={}, will_flag={}, username={:?}",
            connect.client_id, connect.clean_session, connect.will_flag, connect.username
        );

        // Validate protocol name and version
        if connect.protocol_name != v3::PROTOCOL_NAME
            || connect.protocol_level != v3::PROTOCOL_LEVEL
        {
            info!(
                "Invalid protocol: name={}, level={}",
                connect.protocol_name, connect.protocol_level
            );
            self.send_connack_error(
                &mut stream,
                v3::connect_return_codes::UNACCEPTABLE_PROTOCOL_VERSION,
            )
            .await?;
            return Ok(State::Cleanup);
        }

        // Validate client ID
        if let Err(return_code) =
            Self::validate_client_id(&connect.client_id, connect.clean_session)
        {
            info!("Client ID validation failed: {}", connect.client_id);
            self.send_connack_error(&mut stream, return_code).await?;
            return Ok(State::Cleanup);
        }

        // Authenticate the client
        let credentials = crate::auth::Credentials {
            username: connect.username.clone(),
            password: connect.password.clone(),
            client_id: connect.client_id.clone(),
        };

        match self.broker.authenticator().authenticate(&credentials).await {
            Ok(()) => {}
            Err(crate::auth::AuthError::InvalidCredentials) => {
                info!(
                    "Authentication failed for client_id={}, username={:?}",
                    connect.client_id, connect.username
                );
                self.send_connack_error(
                    &mut stream,
                    v3::connect_return_codes::BAD_USERNAME_OR_PASSWORD,
                )
                .await?;
                return Ok(State::Cleanup);
            }
            Err(e) => {
                error!(
                    "Authentication error for client_id={}: {}",
                    connect.client_id, e
                );
                self.send_connack_error(&mut stream, v3::connect_return_codes::NOT_AUTHORIZED)
                    .await?;
                return Ok(State::Cleanup);
            }
        }

        // Update session parameters
        self.clean_session = connect.clean_session;
        self.keep_alive = connect.keep_alive;

        // Store Will message if present
        if connect.will_flag {
            if let (Some(topic), Some(payload)) = (connect.will_topic, connect.will_payload) {
                let will = WillMessage {
                    topic,
                    payload,
                    qos: connect.will_qos,
                    retain: connect.will_retain,
                };
                self.will_message.replace(will);
                info!("Stored Will message for session {}", self.id);
            }
        }

        // Handle client ID and potential session takeover
        let mut session_present = false;

        if !connect.client_id.is_empty() {
            self.client_id.replace(connect.client_id.clone());

            // Check client ID collision and do take over
            if let Some(takeover) = self
                .broker
                .has_collision(&connect.client_id, self.takeover.clone())
                .await
            {
                info!(
                    "Client {} already connected, initiating takeover",
                    connect.client_id
                );

                takeover(stream, connect.clean_session, connect.keep_alive).await;
                return Ok(State::Cleanup); // Exit this session task
            }

            // Handle session persistence
            if connect.clean_session {
                // Clean session: delete any existing saved session
                if let Err(e) = self.broker.storage().delete_session(&connect.client_id) {
                    warn!(
                        "Failed to delete existing session for {}: {}",
                        connect.client_id, e
                    );
                }
            } else {
                // Persistent session: try to restore from storage
                session_present = self.restore_from_storage(&connect.client_id).await;
            }
        }

        // Send CONNACK response
        let connack = packet::ConnAckPacket {
            session_present,
            return_code: v3::connect_return_codes::ACCEPTED,
        };
        send_packet(packet::Packet::ConnAck(connack), &mut stream).await?;

        self.stream.replace(stream);
        Ok(State::Processing)
    }

    /// Validate client ID according to MQTT 3.1.1 spec
    fn validate_client_id(client_id: &str, clean_session: bool) -> std::result::Result<(), u8> {
        if !client_id.is_empty() {
            // Check length (23 UTF-8 bytes max)
            if client_id.len() > 23 {
                return Err(v3::connect_return_codes::IDENTIFIER_REJECTED);
            }
            // Check character set (0-9, a-z, A-Z)
            if !client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(v3::connect_return_codes::IDENTIFIER_REJECTED);
            }
        } else if !clean_session {
            // Empty client ID with clean_session=false is not allowed
            return Err(v3::connect_return_codes::IDENTIFIER_REJECTED);
        }
        Ok(())
    }

    /// Send CONNACK with error return code
    async fn send_connack_error(
        &self,
        stream: &mut Box<dyn AsyncStream>,
        return_code: u8,
    ) -> Result<()> {
        let connack = packet::ConnAckPacket {
            session_present: false,
            return_code,
        };
        send_packet(packet::Packet::ConnAck(connack), stream).await
    }

    async fn on_takeover(
        &mut self,
        stream: &mut Box<dyn AsyncStream>,
        clean_session: bool,
        keep_alive: u16,
    ) -> Result<()> {
        // Session present is true if clean_session=false (persistent session was resumed)
        let session_present = !clean_session;
        let connack = packet::ConnAckPacket {
            session_present,
            return_code: v3::connect_return_codes::ACCEPTED,
        };
        send_packet(packet::Packet::ConnAck(connack), stream).await?;

        self.clean_session = clean_session;
        self.keep_alive = keep_alive;
        if clean_session {
            self.broker.unsubscribe_all(&self.id).await;
            // Delete any saved session from storage
            if let Some(client_id) = &self.client_id {
                if let Err(e) = self.broker.storage().delete_session(client_id) {
                    warn!("Failed to delete session {} from storage: {}", client_id, e);
                }
            }
            // Clear in-flight messages
            self.inflight_queue.clear();
        }
        Ok(())
    }

    async fn on_packet<W: AsyncWrite + Unpin>(
        &mut self,
        pack: packet::Packet,
        writer: &mut W,
    ) -> Result<bool> {
        // Read and parse MQTT packet from stream
        match pack {
            packet::Packet::Publish(p) => self.on_publish(p, writer).await,
            packet::Packet::PubAck(p) => self.on_puback(p, writer).await,
            packet::Packet::PubRec(p) => self.on_pubrec(p, writer).await,
            packet::Packet::PubRel(p) => self.on_pubrel(p, writer).await,
            packet::Packet::PubComp(p) => self.on_pubcomp(p, writer).await,
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

    async fn on_message<W: AsyncWrite + Unpin>(
        &mut self,
        message: packet::Packet,
        writer: &mut W,
    ) -> Result<()> {
        if let packet::Packet::Publish(mut pub_packet) = message {
            if pub_packet.qos == packet::QoS::AtMostOnce {
                // QoS=0: Just send it
                send_packet(packet::Packet::Publish(pub_packet), writer).await?;
            } else if pub_packet.qos == packet::QoS::AtLeastOnce {
                if !pub_packet.dup {
                    // QoS=1 without DUP: Assign packet ID and track it
                    let packet_id = self.next_packet_id();
                    pub_packet.packet_id = Some(packet_id);

                    debug!("Sending new QoS=1 message with packet_id={packet_id}");

                    // Send it immediately
                    send_packet(packet::Packet::Publish(pub_packet.clone()), writer).await?;

                    // Add to in-flight queue with DUP flag for retransmission
                    pub_packet.dup = true;
                    let inflight = InflightMessage {
                        packet: pub_packet,
                        timestamp: Instant::now(),
                        retry_count: 0,
                        qos2_state: None, // QoS=1
                    };
                    self.inflight_queue.push_back(inflight);
                } else {
                    // QoS=1 with DUP: Just send it
                    send_packet(packet::Packet::Publish(pub_packet), writer).await?;
                }
            } else if pub_packet.qos == packet::QoS::ExactlyOnce {
                if !pub_packet.dup {
                    // QoS=2 without DUP: Assign packet ID and track it
                    let packet_id = self.next_packet_id();
                    pub_packet.packet_id = Some(packet_id);

                    debug!("Sending new QoS=2 message with packet_id={packet_id}");

                    // Send it immediately
                    send_packet(packet::Packet::Publish(pub_packet.clone()), writer).await?;

                    // Add to in-flight queue with DUP flag and QoS=2 state
                    pub_packet.dup = true;
                    let inflight = InflightMessage {
                        packet: pub_packet,
                        timestamp: Instant::now(),
                        retry_count: 0,
                        qos2_state: Some(Qos2State::AwaitingPubRec),
                    };
                    self.inflight_queue.push_back(inflight);
                } else {
                    // QoS=2 with DUP: Just send it (retransmission)
                    send_packet(packet::Packet::Publish(pub_packet), writer).await?;
                }
            }
        }
        Ok(())
    }

    async fn on_puback<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::PubAckPacket,
        _writer: &mut W,
    ) -> Result<bool> {
        info!(
            "PUBACK received for session {}: packet_id={}",
            self.id, packet.packet_id
        );

        // Find and remove the acknowledged message from in-flight queue
        self.inflight_queue
            .retain(|inflight| inflight.packet.packet_id != Some(packet.packet_id));

        debug!(
            "Removed acknowledged message, {} messages still in-flight",
            self.inflight_queue.len()
        );

        Ok(true)
    }

    /// Handle PUBREC from client (we are sender, client acknowledged receipt)
    async fn on_pubrec<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::PubRecPacket,
        writer: &mut W,
    ) -> Result<bool> {
        info!(
            "PUBREC received for session {}: packet_id={}",
            self.id, packet.packet_id
        );

        // Find the message in inflight queue and update state to AwaitingPubComp
        let mut found = false;
        for inflight in self.inflight_queue.iter_mut() {
            if inflight.packet.packet_id == Some(packet.packet_id)
                && inflight.qos2_state == Some(Qos2State::AwaitingPubRec)
            {
                inflight.qos2_state = Some(Qos2State::AwaitingPubComp);
                inflight.timestamp = Instant::now(); // Reset for PUBREL retransmission
                inflight.retry_count = 0;
                found = true;
                break;
            }
        }

        if !found {
            // Could be a duplicate PUBREC for a message we already processed
            debug!(
                "PUBREC for unknown or already-processed packet_id={}",
                packet.packet_id
            );
        }

        // Send PUBREL regardless (idempotent response)
        let pubrel = packet::PubRelPacket {
            packet_id: packet.packet_id,
        };
        send_packet(packet::Packet::PubRel(pubrel), writer).await?;
        debug!(
            "Sent PUBREL for packet_id={} to session {}",
            packet.packet_id, self.id
        );

        Ok(true)
    }

    /// Handle PUBREL from client (client is releasing message, we should route it)
    async fn on_pubrel<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::PubRelPacket,
        writer: &mut W,
    ) -> Result<bool> {
        info!(
            "PUBREL received for session {}: packet_id={}",
            self.id, packet.packet_id
        );

        // Send PUBCOMP first (even if we don't have the message, for idempotency)
        let pubcomp = packet::PubCompPacket {
            packet_id: packet.packet_id,
        };
        send_packet(packet::Packet::PubComp(pubcomp), writer).await?;
        debug!(
            "Sent PUBCOMP for packet_id={} to session {}",
            packet.packet_id, self.id
        );

        // Remove message from inbound_qos2 and route it
        if let Some(received) = self.inbound_qos2.remove(&packet.packet_id) {
            // Check publish authorization before routing
            let client_id = self.client_id.as_deref().unwrap_or("");
            if self
                .broker
                .authorizer()
                .authorize_publish(client_id, &received.packet.topic)
                .await
                .is_err()
            {
                info!(
                    "QoS=2 PUBLISH denied by ACL for client_id={}, topic={}",
                    client_id, received.packet.topic
                );
                return Ok(true); // Silently drop, don't route
            }

            // Route the message
            self.broker.route(received.packet).await;
            debug!(
                "Routed QoS=2 message for packet_id={} from session {}",
                packet.packet_id, self.id
            );
        } else {
            // Already processed or unknown - normal for retransmissions
            debug!(
                "PUBREL for unknown packet_id={}, already processed",
                packet.packet_id
            );
        }

        Ok(true)
    }

    /// Handle PUBCOMP from client (QoS=2 complete, we can remove from inflight)
    async fn on_pubcomp<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::PubCompPacket,
        _writer: &mut W,
    ) -> Result<bool> {
        info!(
            "PUBCOMP received for session {}: packet_id={}",
            self.id, packet.packet_id
        );

        // Remove the message from inflight queue
        self.inflight_queue
            .retain(|inflight| inflight.packet.packet_id != Some(packet.packet_id));

        debug!(
            "QoS=2 exchange complete, {} messages still in-flight",
            self.inflight_queue.len()
        );

        Ok(true)
    }

    async fn on_publish<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::PublishPacket,
        writer: &mut W,
    ) -> Result<bool> {
        info!(
            "PUBLISH received for session {}: topic={}, qos={:?}, retain={}, dup={}",
            self.id, packet.topic, packet.qos, packet.retain, packet.dup
        );

        // Handle QoS=1: Send PUBACK if required
        if packet.qos == packet::QoS::AtLeastOnce {
            if let Some(packet_id) = packet.packet_id {
                // Always send PUBACK for QoS=1 (even for duplicates)
                let puback = packet::PubAckPacket { packet_id };
                send_packet(packet::Packet::PubAck(puback), writer).await?;
                debug!(
                    "Sent PUBACK for packet_id={} to session {}",
                    packet_id, self.id
                );

                // Check DUP flag - if it's a duplicate, don't route
                if packet.dup {
                    info!(
                        "Received duplicate PUBLISH with packet_id={}, not routing",
                        packet_id
                    );
                    return Ok(true); // Still return success, just don't route
                }
            } else {
                // QoS=1 requires packet_id
                error!("QoS=1 PUBLISH missing packet_id from session {}", self.id);
                return Ok(false); // Disconnect on protocol violation
            }
        }

        // Handle QoS=2: Store message and send PUBREC, wait for PUBREL to route
        if packet.qos == packet::QoS::ExactlyOnce {
            if let Some(packet_id) = packet.packet_id {
                // Check if this is a duplicate (we already have this packet_id)
                if self.inbound_qos2.contains_key(&packet_id) {
                    // Duplicate: resend PUBREC
                    let pubrec = packet::PubRecPacket { packet_id };
                    send_packet(packet::Packet::PubRec(pubrec), writer).await?;
                    debug!(
                        "Resent PUBREC for duplicate QoS=2 packet_id={} to session {}",
                        packet_id, self.id
                    );
                    return Ok(true);
                }

                // Store message for later routing when PUBREL arrives
                self.inbound_qos2.insert(
                    packet_id,
                    ReceivedQos2Message {
                        packet: packet.clone(),
                    },
                );

                // Send PUBREC
                let pubrec = packet::PubRecPacket { packet_id };
                send_packet(packet::Packet::PubRec(pubrec), writer).await?;
                debug!(
                    "Sent PUBREC for packet_id={} to session {}",
                    packet_id, self.id
                );

                // Don't route yet - wait for PUBREL
                return Ok(true);
            } else {
                // QoS=2 requires packet_id
                error!("QoS=2 PUBLISH missing packet_id from session {}", self.id);
                return Ok(false); // Disconnect on protocol violation
            }
        }

        // Check publish authorization
        let client_id = self.client_id.as_deref().unwrap_or("");
        if self
            .broker
            .authorizer()
            .authorize_publish(client_id, &packet.topic)
            .await
            .is_err()
        {
            info!(
                "PUBLISH denied by ACL for client_id={}, topic={}",
                client_id, packet.topic
            );
            return Ok(true); // Silently drop, don't route, don't disconnect
        }

        // Route the message through the router (only for non-duplicates or QoS=0)
        self.broker.route(packet).await;

        Ok(true)
    }

    async fn on_subscribe<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::SubscribePacket,
        writer: &mut W,
    ) -> Result<bool> {
        info!(
            "SUBSCRIBE received for session {}: topics={:?}",
            self.id, packet.topic_filters
        );

        // Check subscribe authorization per topic filter
        let client_id = self.client_id.as_deref().unwrap_or("");
        let mut authorized_filters = Vec::new();
        let mut denied_indices = Vec::new();

        for (i, (topic_filter, qos)) in packet.topic_filters.iter().enumerate() {
            if self
                .broker
                .authorizer()
                .authorize_subscribe(client_id, topic_filter)
                .await
                .is_ok()
            {
                authorized_filters.push((topic_filter.clone(), *qos));
            } else {
                info!(
                    "SUBSCRIBE denied by ACL for client_id={}, topic_filter={}",
                    client_id, topic_filter
                );
                denied_indices.push(i);
            }
        }

        // Subscribe only the authorized filters
        let (router_codes, retained_messages) = if !authorized_filters.is_empty() {
            self.broker
                .subscribe(&self.id, self.message_tx.clone(), &authorized_filters)
                .await
        } else {
            (vec![], vec![])
        };

        // Reconstruct return_codes with FAILURE for denied topics
        let mut return_codes = Vec::with_capacity(packet.topic_filters.len());
        let mut router_idx = 0;
        for i in 0..packet.topic_filters.len() {
            if denied_indices.contains(&i) {
                return_codes.push(v3::subscribe_return_codes::FAILURE);
            } else {
                return_codes.push(router_codes[router_idx]);
                router_idx += 1;
            }
        }

        // Send SUBACK
        let suback = packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes,
        };
        send_packet(packet::Packet::SubAck(suback), writer).await?;

        // Then send any retained messages
        for retained_msg in retained_messages {
            self.message_tx
                .send(packet::Packet::Publish(retained_msg))
                .await?;
        }

        Ok(true)
    }

    async fn on_unsubscribe<W: AsyncWrite + Unpin>(
        &mut self,
        packet: packet::UnsubscribePacket,
        writer: &mut W,
    ) -> Result<bool> {
        info!(
            "UNSUBSCRIBE received for session {}: topics={:?}",
            self.id, packet.topic_filters
        );

        // Handle unsubscription logic through router
        self.broker
            .unsubscribe(&self.id, &packet.topic_filters)
            .await;

        // Send UNSUBACK
        let unsuback = packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        send_packet(packet::Packet::UnsubAck(unsuback), writer).await?;
        Ok(true)
    }

    async fn on_pingreq<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<bool> {
        debug!("PINGREQ received for session {}", self.id);
        send_packet(packet::Packet::PingResp, writer).await?;
        Ok(true)
    }

    async fn on_disconnect(&mut self) -> Result<bool> {
        info!("DISCONNECT received for session {}", self.id);
        // Clear Will message on normal disconnect
        self.will_message.take();
        Ok(false)
    }

    async fn on_retransmit<W: AsyncWrite + Unpin>(&mut self, writer: &mut W) -> Result<()> {
        // Get config values
        let retransmission_interval_ms = self.broker.config().get_retransmission_interval_ms();
        if retransmission_interval_ms == 0 {
            return Ok(()); // Retransmission disabled
        }

        let max_retransmission_limit = self.broker.config().max_retransmission_limit;
        let now = Instant::now();
        let mut publish_to_retransmit = Vec::new();
        let mut pubrel_to_retransmit = Vec::new();

        // Update retry counts and collect messages to retransmit
        self.inflight_queue.retain_mut(|inflight| {
            let elapsed_ms = now.duration_since(inflight.timestamp).as_millis() as u64;

            if elapsed_ms >= retransmission_interval_ms {
                inflight.retry_count += 1;
                inflight.timestamp = now;

                if inflight.retry_count > max_retransmission_limit {
                    info!(
                        "Message packet_id={:?} exceeded max retransmission limit ({}), dropping",
                        inflight.packet.packet_id, max_retransmission_limit
                    );
                    false // Remove from queue
                } else {
                    // Determine what to retransmit based on QoS level and state
                    match inflight.qos2_state {
                        None | Some(Qos2State::AwaitingPubRec) => {
                            // QoS=1 or QoS=2 waiting for PUBREC: retransmit PUBLISH
                            debug!(
                                "Retransmitting PUBLISH packet_id={:?}, retry_count={}/{}",
                                inflight.packet.packet_id,
                                inflight.retry_count,
                                max_retransmission_limit
                            );
                            publish_to_retransmit.push(inflight.packet.clone());
                        }
                        Some(Qos2State::AwaitingPubComp) => {
                            // QoS=2 waiting for PUBCOMP: retransmit PUBREL
                            if let Some(packet_id) = inflight.packet.packet_id {
                                debug!(
                                    "Retransmitting PUBREL packet_id={}, retry_count={}/{}",
                                    packet_id, inflight.retry_count, max_retransmission_limit
                                );
                                pubrel_to_retransmit.push(packet_id);
                            }
                        }
                    }
                    true // Keep in queue
                }
            } else {
                true // Keep in queue, not time to retry yet
            }
        });

        // Retransmit PUBLISH messages via mailbox (will be sent by on_message)
        for packet in publish_to_retransmit {
            self.message_tx
                .send(packet::Packet::Publish(packet))
                .await?;
        }

        // Retransmit PUBREL messages directly
        for packet_id in pubrel_to_retransmit {
            let pubrel = packet::PubRelPacket { packet_id };
            send_packet(packet::Packet::PubRel(pubrel), writer).await?;
        }

        Ok(())
    }

    async fn publish_will(&mut self) {
        if let Some(will) = self.will_message.take() {
            info!(
                "Publishing Will message for session {}: topic={}",
                self.id, will.topic
            );
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

    /// Save session state to storage for clean_session=false clients
    async fn save_to_storage(&self) {
        let client_id = match &self.client_id {
            Some(id) => id,
            None => return, // No client ID, nothing to save
        };

        let now = Utc::now();

        // Build persisted session
        let persisted_session = PersistedSession {
            client_id: client_id.clone(),
            next_packet_id: self.next_packet_id,
            keep_alive: self.keep_alive,
            will_message: self.will_message.as_ref().map(|w| PersistedWillMessage {
                topic: w.topic.clone(),
                payload: w.payload.clone(),
                qos: StoredQoS::from(w.qos),
                retain: w.retain,
            }),
            created_at: now,
            updated_at: now,
        };

        // Get subscriptions from router
        let router_subs = self.broker.get_subscriptions(&self.id).await;
        let persisted_subs: Vec<PersistedSubscription> = router_subs
            .into_iter()
            .map(|(filter, qos)| PersistedSubscription {
                client_id: client_id.clone(),
                topic_filter: filter,
                qos: StoredQoS::from(qos),
            })
            .collect();

        // Convert in-flight messages
        let persisted_inflight: Vec<PersistedInflightMessage> = self
            .inflight_queue
            .iter()
            .map(|inflight| {
                let qos2_state = inflight.qos2_state.map(|s| match s {
                    Qos2State::AwaitingPubRec => PersistedQos2State::AwaitingPubRec,
                    Qos2State::AwaitingPubComp => PersistedQos2State::AwaitingPubComp,
                });
                PersistedInflightMessage {
                    client_id: client_id.clone(),
                    packet_id: inflight.packet.packet_id.unwrap_or(0),
                    topic: inflight.packet.topic.clone(),
                    payload: inflight.packet.payload.clone(),
                    qos: StoredQoS::from(inflight.packet.qos),
                    retain: inflight.packet.retain,
                    retry_count: inflight.retry_count,
                    qos2_state,
                    created_at: now,
                }
            })
            .collect();

        // Convert inbound QoS=2 messages
        let persisted_inbound_qos2: Vec<PersistedInboundQos2Message> = self
            .inbound_qos2
            .iter()
            .map(|(&packet_id, received)| PersistedInboundQos2Message {
                client_id: client_id.clone(),
                packet_id,
                topic: received.packet.topic.clone(),
                payload: received.packet.payload.clone(),
                retain: received.packet.retain,
                received_at: now,
            })
            .collect();

        // Save to storage
        if let Err(e) = self.broker.storage().save_session(
            &persisted_session,
            &persisted_subs,
            &persisted_inflight,
            &persisted_inbound_qos2,
        ) {
            warn!("Failed to save session {} to storage: {}", client_id, e);
        } else {
            info!(
                "Saved session {} to storage ({} subscriptions, {} inflight, {} inbound qos2)",
                client_id,
                persisted_subs.len(),
                persisted_inflight.len(),
                persisted_inbound_qos2.len()
            );
        }
    }

    /// Load session state from storage and restore subscriptions/inflight messages
    async fn restore_from_storage(&mut self, client_id: &str) -> bool {
        // Load session
        let persisted_session = match self.broker.storage().load_session(client_id) {
            Ok(Some(session)) => session,
            Ok(None) => return false, // No saved session
            Err(e) => {
                warn!("Failed to load session {} from storage: {}", client_id, e);
                return false;
            }
        };

        // Load subscriptions
        let persisted_subs = match self.broker.storage().load_subscriptions(client_id) {
            Ok(subs) => subs,
            Err(e) => {
                warn!(
                    "Failed to load subscriptions for {} from storage: {}",
                    client_id, e
                );
                Vec::new()
            }
        };

        // Load inflight messages
        let persisted_inflight = match self.broker.storage().load_inflight_messages(client_id) {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!(
                    "Failed to load inflight messages for {} from storage: {}",
                    client_id, e
                );
                Vec::new()
            }
        };

        // Load inbound QoS=2 messages
        let persisted_inbound_qos2 = match self.broker.storage().load_inbound_qos2_messages(client_id) {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!(
                    "Failed to load inbound QoS=2 messages for {} from storage: {}",
                    client_id, e
                );
                Vec::new()
            }
        };

        // Restore session state
        self.next_packet_id = persisted_session.next_packet_id;
        self.keep_alive = persisted_session.keep_alive;
        self.will_message = persisted_session.will_message.map(|w| WillMessage {
            topic: w.topic,
            payload: w.payload,
            qos: packet::QoS::from(w.qos),
            retain: w.retain,
        });

        // Restore subscriptions to router
        let topic_filters: Vec<(String, packet::QoS)> = persisted_subs
            .iter()
            .map(|s| (s.topic_filter.clone(), packet::QoS::from(s.qos)))
            .collect();

        if !topic_filters.is_empty() {
            self.broker
                .subscribe(&self.id, self.message_tx.clone(), &topic_filters)
                .await;
            info!(
                "Restored {} subscriptions for session {}",
                topic_filters.len(),
                client_id
            );
        }

        // Restore in-flight messages
        for msg in persisted_inflight {
            // Convert persisted qos2_state to runtime state
            let qos2_state = msg.qos2_state.map(|s| match s {
                PersistedQos2State::AwaitingPubRec => Qos2State::AwaitingPubRec,
                PersistedQos2State::AwaitingPubComp => Qos2State::AwaitingPubComp,
            });

            let inflight = InflightMessage {
                packet: packet::PublishPacket {
                    topic: msg.topic,
                    packet_id: Some(msg.packet_id),
                    payload: msg.payload,
                    qos: packet::QoS::from(msg.qos),
                    retain: msg.retain,
                    dup: true, // Retransmitted messages have DUP set
                },
                timestamp: Instant::now(),
                retry_count: msg.retry_count,
                qos2_state,
            };
            self.inflight_queue.push_back(inflight);
        }

        if !self.inflight_queue.is_empty() {
            info!(
                "Restored {} inflight messages for session {}",
                self.inflight_queue.len(),
                client_id
            );
        }

        // Restore inbound QoS=2 messages
        for msg in persisted_inbound_qos2 {
            self.inbound_qos2.insert(
                msg.packet_id,
                ReceivedQos2Message {
                    packet: packet::PublishPacket {
                        topic: msg.topic,
                        packet_id: Some(msg.packet_id),
                        payload: msg.payload,
                        qos: packet::QoS::ExactlyOnce,
                        retain: msg.retain,
                        dup: false,
                    },
                },
            );
        }

        if !self.inbound_qos2.is_empty() {
            info!(
                "Restored {} inbound QoS=2 messages for session {}",
                self.inbound_qos2.len(),
                client_id
            );
        }

        // Delete from storage after successful restore
        if let Err(e) = self.broker.storage().delete_session(client_id) {
            warn!(
                "Failed to delete restored session {} from storage: {}",
                client_id, e
            );
        }

        true
    }
}
