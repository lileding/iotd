use anyhow::Result;
use bytes::BytesMut;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::protocol::{
    v3, ConnAckPacket, ConnectPacket, Packet, PublishPacket, QoS, SubAckPacket, SubscribePacket,
    UnsubAckPacket, UnsubscribePacket,
};
use crate::server::broker::Broker;
use crate::server::session::Session;

pub struct ClientHandler {
    stream: TcpStream,
    broker: Arc<Broker>,
    session: Option<Session>,
    buffer: BytesMut,
}

impl ClientHandler {
    pub fn new(stream: TcpStream, broker: Arc<Broker>) -> Self {
        Self {
            stream,
            broker,
            session: None,
            buffer: BytesMut::with_capacity(4096),
        }
    }

    pub async fn handle(&mut self) -> Result<()> {
        loop {
            // Handle incoming packets
            match self.read_packet().await {
                Ok(Some(packet)) => {
                    if let Err(e) = self.process_packet(packet).await {
                        error!("Error processing packet: {}", e);
                        break;
                    }
                }
                Ok(None) => {
                    // This shouldn't happen with the new read logic
                    continue;
                }
                Err(e) => {
                    error!("Error reading packet: {}", e);
                    break;
                }
            }

            // Handle broker messages only if session exists
            if self.session.is_some() {
                if let Err(e) = self.handle_broker_messages().await {
                    debug!("Broker message handling stopped: {}", e);
                    break;
                }
            }
        }

        // Cleanup on disconnect
        if let Some(session) = &self.session {
            info!("Client {} disconnected", session.client_id);
            self.broker.unregister_client(&session.client_id);
        }

        Ok(())
    }

    async fn read_packet(&mut self) -> Result<Option<Packet>> {
        // Try to decode existing buffer first
        match Packet::decode(&mut self.buffer) {
            Ok(Some(packet)) => {
                return Ok(Some(packet));
            },
            Ok(None) => {}, // Need more data
            Err(e) => return Err(anyhow::anyhow!("Packet decode error: {}", e)),
        }

        // Read more data into buffer
        let mut temp_buf = [0u8; 1024];
        match self.stream.read(&mut temp_buf).await {
            Ok(0) => Err(anyhow::anyhow!("Connection closed")),
            Ok(n) => {
                self.buffer.extend_from_slice(&temp_buf[..n]);
                // Try to decode again
                match Packet::decode(&mut self.buffer) {
                    Ok(packet) => Ok(packet),
                    Err(e) => Err(anyhow::anyhow!("Packet decode error: {}", e)),
                }
            }
            Err(e) => Err(anyhow::anyhow!("Read error: {}", e)),
        }
    }

    async fn process_packet(&mut self, packet: Packet) -> Result<()> {
        match packet {
            Packet::Connect(connect) => self.handle_connect(connect).await,
            Packet::Publish(publish) => self.handle_publish(publish).await,
            Packet::Subscribe(subscribe) => self.handle_subscribe(subscribe).await,
            Packet::Unsubscribe(unsubscribe) => self.handle_unsubscribe(unsubscribe).await,
            Packet::PingReq => self.handle_ping().await,
            Packet::Disconnect => {
                info!("Client requested disconnect");
                Err(anyhow::anyhow!("Client disconnected"))
            }
            _ => {
                warn!("Unhandled packet type: {:?}", packet);
                Ok(())
            }
        }
    }

    async fn handle_connect(&mut self, connect: ConnectPacket) -> Result<()> {
        debug!("Processing CONNECT packet for client: {}", connect.client_id);

        // Validate protocol
        let return_code = if connect.protocol_name != v3::PROTOCOL_NAME {
            v3::connect_return_codes::UNACCEPTABLE_PROTOCOL_VERSION
        } else if connect.protocol_level != v3::PROTOCOL_LEVEL {
            v3::connect_return_codes::UNACCEPTABLE_PROTOCOL_VERSION
        } else {
            v3::connect_return_codes::ACCEPTED
        };

        let connack = ConnAckPacket {
            session_present: false, // Always false for clean session in Stone 1
            return_code,
        };

        debug!("Sending CONNACK to client: {}", connect.client_id);
        self.send_packet(Packet::ConnAck(connack)).await?;

        if return_code == v3::connect_return_codes::ACCEPTED {
            // Create session
            let session = Session::new(connect.client_id.clone(), connect.clean_session, connect.keep_alive);
            let message_receiver = self.broker.register_client(session.client_id.clone());
            
            let mut session = session;
            session.set_message_receiver(message_receiver);
            
            info!("Client {} connected successfully", session.client_id);
            self.session = Some(session);
        }

        Ok(())
    }

    async fn handle_publish(&mut self, publish: PublishPacket) -> Result<()> {
        debug!("Processing PUBLISH packet for topic: {}", publish.topic);

        // For Stone 1, we only support QoS 0
        if !matches!(publish.qos, QoS::AtMostOnce) {
            warn!("QoS {:?} not supported in Stone 1", publish.qos);
            return Ok(());
        }

        // Publish message through broker
        self.broker.publish(&publish);

        // QoS 0 doesn't require acknowledgment
        Ok(())
    }

    async fn handle_subscribe(&mut self, subscribe: SubscribePacket) -> Result<()> {
        debug!("Processing SUBSCRIBE packet with {} topics", subscribe.topic_filters.len());

        let session = self.session.as_mut().ok_or_else(|| {
            anyhow::anyhow!("No session established")
        })?;

        let mut return_codes = Vec::new();

        let topic_filters = subscribe.topic_filters.clone();
        
        for (topic_filter, requested_qos) in &subscribe.topic_filters {
            // For Stone 1, we only support QoS 0
            let granted_qos = match requested_qos {
                QoS::AtMostOnce => v3::subscribe_return_codes::MAXIMUM_QOS_0,
                _ => {
                    warn!("QoS {:?} not supported, granting QoS 0", requested_qos);
                    v3::subscribe_return_codes::MAXIMUM_QOS_0
                }
            };

            // Subscribe through broker 
            let _retained_messages = self.broker.subscribe(session.client_id.clone(), topic_filter.clone());
            
            // Add to session subscriptions
            session.add_subscription(topic_filter.clone(), granted_qos);
            
            return_codes.push(granted_qos);
        }

        // Send SUBACK first
        let suback = SubAckPacket {
            packet_id: subscribe.packet_id,
            return_codes,
        };
        self.send_packet(Packet::SubAck(suback)).await?;

        // Then send retained messages for all subscriptions
        for (topic_filter, _) in &topic_filters {
            let retained_messages = self.broker.get_retained_messages(topic_filter);
            for message in retained_messages {
                let publish_packet = PublishPacket {
                    topic: message.topic,
                    packet_id: None, // QoS 0
                    payload: message.payload,
                    qos: QoS::AtMostOnce,
                    retain: false, // Don't set retain flag when delivering retained messages
                    dup: false,
                };
                self.send_packet(Packet::Publish(publish_packet)).await?;
            }
        }
        Ok(())
    }

    async fn handle_unsubscribe(&mut self, unsubscribe: UnsubscribePacket) -> Result<()> {
        debug!("Processing UNSUBSCRIBE packet with {} topics", unsubscribe.topic_filters.len());

        let session = self.session.as_mut().ok_or_else(|| {
            anyhow::anyhow!("No session established")
        })?;

        for topic_filter in unsubscribe.topic_filters {
            self.broker.unsubscribe(&session.client_id, &topic_filter);
            session.remove_subscription(&topic_filter);
        }

        let unsuback = UnsubAckPacket {
            packet_id: unsubscribe.packet_id,
        };

        self.send_packet(Packet::UnsubAck(unsuback)).await?;
        Ok(())
    }

    async fn handle_ping(&mut self) -> Result<()> {
        debug!("Processing PINGREQ packet");
        self.send_packet(Packet::PingResp).await
    }

    async fn handle_broker_messages(&mut self) -> Result<()> {
        let session = match &mut self.session {
            Some(session) => session,
            None => return Ok(()), // No session yet
        };

        let receiver = match &mut session.message_receiver {
            Some(receiver) => receiver,
            None => return Ok(()),
        };

        match timeout(Duration::from_millis(100), receiver.recv()).await {
            Ok(Ok(message)) => {
                let publish_packet = PublishPacket {
                    topic: message.topic,
                    packet_id: None, // QoS 0 for Stone 1
                    payload: message.payload,
                    qos: QoS::AtMostOnce,
                    retain: false,
                    dup: false,
                };

                self.send_packet(Packet::Publish(publish_packet)).await?;
                Ok(())
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                Err(anyhow::anyhow!("Broker channel closed"))
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                warn!("Client lagging behind, some messages may be dropped");
                Ok(())
            }
            Err(_) => Ok(()), // Timeout, no message
        }
    }

    async fn send_packet(&mut self, packet: Packet) -> Result<()> {
        let mut buf = BytesMut::new();
        packet.encode(&mut buf);
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }
}