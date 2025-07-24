use crate::{
    broker::Broker,
    protocol::packet, protocol::v3,
    transport::AsyncStream,
};
use std::sync::{atomic::{AtomicBool, AtomicU16, Ordering}, Arc};
use tokio::{io::AsyncWriteExt, sync::{mpsc, Mutex}, task::JoinHandle};
use tracing::{debug, info, error};
use uuid::Uuid;
use thiserror::Error;

pub struct Session {
    id: String,
    client_id: Mutex<Option<String>>,
    clean_session: AtomicBool,
    keep_alive: AtomicU16,
    broker: Arc<Broker>,
    stream: Mutex<Option<Box<dyn AsyncStream>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    command_tx: mpsc::Sender<Command>,
    command_rx: Mutex<mpsc::Receiver<Command>>,
    message_tx: mpsc::Sender<packet::Packet>,
    message_rx: Mutex<Option<mpsc::Receiver<packet::Packet>>>,
}

pub type Mailbox = mpsc::Sender<packet::Packet>;

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

impl Session {
    pub async fn spawn(broker: Arc<Broker>, stream: Box<dyn AsyncStream>) -> Arc<Session> {
        let id = Uuid::new_v4().to_string();
        let (command_tx, command_rx) = mpsc::channel(10);
        let (message_tx, message_rx) = mpsc::channel(100);

        let sess = Arc::new(Self {
            id: id.clone(),
            client_id: Mutex::new(None),
            clean_session: AtomicBool::new(true),
            keep_alive: AtomicU16::new(0),
            broker: Arc::clone(&broker),
            stream: Mutex::new(Some(stream)),
            task: Mutex::new(None),
            command_tx,
            command_rx: Mutex::new(command_rx),
            message_tx,
            message_rx: Mutex::new(Some(message_rx)),
        });

        let task = Arc::clone(&sess);
        sess.task.lock().await.replace(tokio::spawn(async move {
            task.run().await;
            broker.remove_session(&task).await;
        }));

        sess
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn client_id(&self) -> Option<String> {
        self.client_id.lock().await.clone()
    }

    pub async fn cancel(&self) -> Option<JoinHandle<()>> {
        self.command_tx.send(Command::Cancel).await.unwrap_or_else(|e| {
            info!("Session canceled too early: {}", e);
        });
        self.task.lock().await.take()
    }

    async fn run(&self) {
        let mut state = State::WaitConnect;
        loop {
            state = match state {
                State::WaitConnect => self.do_wait_connect().await,
                State::Processing => self.do_processing().await,
                State::WaitTakeover => self.do_wait_takeover().await,
                State::Cleanup => {
                    debug!("Session {} STATE CLEANUP", self.id());
                    // TODO: Save subscriptions and unfinished messages to persistent storage
                    // This will be implemented in Milestone 3 for persistent session support
                    break;
                },
            }
        }
        debug!("Session {} EXIT RUN", self.id());
    }

    async fn do_wait_connect(&self) -> State {
        debug!("Session {} STATE WAIT_CONNECT", self.id());

        let mut stream = match self.stream.lock().await.take() {
            Some(stream) => stream,
            None => { return State::Cleanup; },
        };
        let mut command_rx = self.command_rx.lock().await;

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

            _ = command_rx.recv() => State::Cleanup
        }
    }

    async fn do_processing(&self) -> State {
        debug!("Session {} STATE PROCESSING", self.id());

        let mut stream = match self.stream.lock().await.take() {
            Some(stream) => stream,
            None => {
                error!("Error processing session {} with no client stream", self.id());
                return State::Cleanup;
            }
        };
        let (mut reader, mut writer) = stream.split();
        let mut message_rx = match self.message_rx.lock().await.take() {
            Some(v) => v,
            None => {
                error!("Error processing session {} with no message queue", self.id());
                return State::Cleanup;
            }
        };
        let mut command_rx = self.command_rx.lock().await;
        let mut next_state = State::Cleanup;

        loop {
            tokio::select! {
                pack = packet::Packet::decode(&mut reader) => {
                    let success = match pack {
                        Ok(pack) => {
                            self.on_packet(pack).await.unwrap_or_else(|e| {
                                info!("Session {} error: {}", self.id(), e);
                                false
                            })
                        },
                        Err(e) => {
                            info!("Session {} error: {}", self.id(), e);
                            false
                        }
                    };
                    if !success {
                        if !self.clean_session.load(Ordering::Acquire) {
                            next_state = State::WaitTakeover;
                        }
                        break;
                    }
                }

                Some(message) = message_rx.recv() => {
                    let mut buf = bytes::BytesMut::new();
                    message.encode(&mut buf);
                    if let Err(e) = writer.write_all(&buf).await {
                        info!("Session {} error: {}", self.id(), e);
                        if !self.clean_session.load(Ordering::Acquire) {
                            next_state = State::WaitTakeover;
                        }
                        break;
                    }
                }

                command = command_rx.recv() => {
                    if let Some(Command::Takeover(
                            mut new_stream, clean_session, keep_alive)) = command {
                        // Disconnect original client
                        let mut buf = bytes::BytesMut::new();
                        packet::Packet::Disconnect.encode(&mut buf);
                        let _ = writer.write_all(&buf).await;

                        // Process takeover message
                        match self.on_takeover(&mut new_stream, clean_session, keep_alive).await {
                            Ok(_) => {
                                self.stream.lock().await.replace(new_stream);
                                next_state = State::Processing;
                            },
                            Err(e) => {
                                info!("Session {} error: {}", self.id(), e);
                                if !clean_session {
                                    next_state = State::WaitTakeover;
                                }
                            }
                        }
                    }

                    // Always break for any reason
                    break;
                }
            }
        }

        self.message_rx.lock().await.replace(message_rx);

        next_state
    }

    async fn do_wait_takeover(&self) -> State {
        debug!("Session {} STATE WAIT_TAKEOVER", self.id());

        let mut command_rx = self.command_rx.lock().await;

        match command_rx.recv().await {
            Some(Command::Takeover(mut stream, clean_session, keep_alive)) => {
                match self.on_takeover(&mut stream, clean_session, keep_alive).await {
                    Ok(_) => {
                        self.stream.lock().await.replace(stream);
                        State::Processing
                    },
                    Err(e) => {
                        info!("Session {} error: {}", self.id(), e);
                        State::WaitTakeover
                    }
                }
            },
            Some(Command::Cancel) => State::Cleanup,
            _ => State::Cleanup,
        }
    }

    async fn on_connect(&self, connect: packet::ConnectPacket, mut stream: Box<dyn AsyncStream>) -> Result<State> {
        info!("CONNECT received: client_id={}, clean_session={}", connect.client_id, connect.clean_session);

        // Check for empty client ID with clean_session=false
        if connect.client_id.is_empty() && !connect.clean_session {
            // Protocol violation - reject connection
            let connack = packet::ConnAckPacket {
                session_present: false,
                return_code: v3::connect_return_codes::IDENTIFIER_REJECTED,
            };
            let mut buf = bytes::BytesMut::new();
            packet::Packet::ConnAck(connack).encode(&mut buf);
            stream.write_all(&buf).await?;
            return Ok(State::Cleanup); // Close connection
        }

        // Update session parameters
        self.clean_session.store(connect.clean_session, Ordering::Release);
        self.keep_alive.store(connect.keep_alive, Ordering::Release);

        // Handle client ID and potential session takeover
        if !connect.client_id.is_empty() {
            self.client_id.lock().await.replace(connect.client_id.clone());

            // Check for existing session or add new one
            if let Some(session) = self.broker.resolve_collision(&connect.client_id, self.id()).await {
                info!("Client {} already connected, initiating takeover", connect.client_id);

                if let Err(e) = session.command_tx.send(
                    Command::Takeover(
                        stream, connect.clean_session, connect.keep_alive)).await {
                    error!("Failed to send takeover message: {}", e);
                }

                return Ok(State::Cleanup); // Exit this session task
            }
        }

        // Send CONNACK response
        let connack = packet::ConnAckPacket {
            session_present: false, // New connection, no existing session
            return_code: crate::protocol::v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        stream.write_all(&buf).await?;

        self.stream.lock().await.replace(stream);

        Ok(State::Processing)
    }

    async fn on_takeover(&self, stream: &mut Box<dyn AsyncStream>, clean_session: bool, keep_alive: u16) -> Result<()> {
        // Session present is true if clean_session=false (persistent session was resumed)
        let session_present = !clean_session;
        let connack = packet::ConnAckPacket {
            session_present,
            return_code: v3::connect_return_codes::ACCEPTED,
        };
        let mut buf = bytes::BytesMut::new();
        packet::Packet::ConnAck(connack).encode(&mut buf);
        match stream.write_all(&buf).await {
            Ok(_) => {
                self.clean_session.store(clean_session, Ordering::Release);
                self.keep_alive.store(keep_alive, Ordering::Release);
                if clean_session {
                    self.broker.unsubscribe_all(self.id()).await;
                }
                Ok(())
            },
            Err(e) => {
                Err(e.into())
            }
        }
    }

    async fn on_packet(&self, pack: packet::Packet) -> Result<bool> {
        // Read and parse MQTT packet from stream
        match pack {
            packet::Packet::Publish(p) => self.on_publish(p).await,
            packet::Packet::Subscribe(p) => self.on_subscribe(p).await,
            packet::Packet::Unsubscribe(p) => self.on_unsubscribe(p).await,
            packet::Packet::PingReq => self.on_pingreq().await,
            packet::Packet::Disconnect => self.on_disconnect().await,
            _ => {
                error!("Unhandled packet type for session {}", self.id());
                Ok(false)
            }
        }
    }

    async fn on_publish(&self, packet: packet::PublishPacket) -> Result<bool> {
        info!("PUBLISH received for session {}: topic={}, qos={:?}", self.id(), packet.topic, packet.qos);

        // Route the message through the router
        self.broker.route(&packet.topic, &packet.payload).await;

        Ok(true)
    }

    async fn on_subscribe(&self, packet: packet::SubscribePacket) -> Result<bool> {
        info!("SUBSCRIBE received for session {}: topics={:?}", self.id(), packet.topic_filters);

        // Handle subscription logic through router
        let return_codes = self.broker.subscribe(
            self.id(),
            self.message_tx.clone(),
            &packet.topic_filters,
        ).await;

        // Send SUBACK
        let suback = packet::SubAckPacket {
            packet_id: packet.packet_id,
            return_codes,
        };
        self.message_tx.send(packet::Packet::SubAck(suback)).await?;

        Ok(true)
    }

    async fn on_unsubscribe(&self, packet: packet::UnsubscribePacket) -> Result<bool> {
        info!("UNSUBSCRIBE received for session {}: topics={:?}", self.id(), packet.topic_filters);

        // Handle unsubscription logic through router
        self.broker.unsubscribe(self.id(), &packet.topic_filters).await;

        // Send UNSUBACK
        let unsuback = packet::UnsubAckPacket {
            packet_id: packet.packet_id,
        };
        self.message_tx.send(packet::Packet::UnsubAck(unsuback)).await?;
        Ok(true)
    }

    async fn on_pingreq(&self) -> Result<bool> {
        info!("PINGREQ received for session {}", self.id());
        self.message_tx.send(packet::Packet::PingResp).await?;
        Ok(true)
    }

    async fn on_disconnect(&self) -> Result<bool> {
        info!("DISCONNECT received for session {}", self.id());
        Ok(false)
    }
}
