# IoTD Architecture Documentation

## Overview

IoTD (IoT Daemon) is a high-performance MQTT v3.1.1 server built in Rust using the Tokio async runtime. The architecture prioritizes scalability, reliability, and maintainability through a modular, event-driven design with strong concurrency guarantees.

## Core Design Principles

### 1. Event-Driven Architecture
- Uses `tokio::select!` for responsive packet handling and system events
- Non-blocking I/O throughout the system
- Async/await patterns for efficient resource utilization

### 2. Race-Condition-Free Design
- `CancellationToken` for coordinated shutdown across all components
- No shared mutable state without proper synchronization
- Clear ownership boundaries between components

### 3. Modular Component Design
- Clean separation of concerns with well-defined interfaces
- Protocol-agnostic transport layer
- Pluggable architecture for future extensibility

### 4. Thread-Safe Operations
- `Arc<RwLock>` for read-heavy workloads (routing tables)
- `DashMap` for high-performance concurrent access
- Lock-free algorithms where possible

## System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Server (Main)                           │
│  - Lifecycle management                                         │
│  - Signal handling (SIGINT/SIGTERM)                           │
│  - Broker orchestration                                        │
│  - Global configuration                                        │
└────────────────────────┬───────────────────────────────────────┘
                         │
┌────────────────────────┴───────────────────────────────────────┐
│                    Broker (per transport)                       │
│  - TCP listener management                                     │
│  - Session spawning                                            │
│  - Client registry (named_clients: DashMap)                   │
│  - Session cleanup coordination                                │
└────────────────────────┬───────────────────────────────────────┘
                         │
┌────────────────────────┴───────────────────────────────────────┐
│                    Session (per client)                         │
│  - Connection state machine                                    │
│  - Packet processing                                           │
│  - Keep-alive monitoring                                       │
│  - Will message storage                                        │
│  - Message queuing (mailbox)                                  │
└────────────────────────┬───────────────────────────────────────┘
                         │
┌────────────────────────┴───────────────────────────────────────┐
│                         Router                                  │
│  - Topic subscription management                               │
│  - Message routing with wildcard support                       │
│  - Retained message storage                                    │
│  - Bidirectional mapping (topics ↔ sessions)                  │
└─────────────────────────────────────────────────────────────────┘
```

## Component Details

### Server (`src/server.rs`)

The Server is the top-level orchestrator responsible for:

- **Lifecycle Management**: Starting/stopping the entire system
- **Signal Handling**: SIGINT (graceful) and SIGTERM (immediate) shutdown
- **Broker Management**: Creating and managing transport-specific brokers
- **Configuration**: Loading and distributing system configuration
- **Coordination**: Managing shutdown tokens and ensuring clean termination

```rust
pub struct Server {
    brokers: Vec<BrokerHandle>,
    shutdown_token: CancellationToken,
    config: ServerConfig,
}
```

### Broker (`src/broker.rs`)

Each Broker manages connections for a specific transport protocol:

- **Connection Acceptance**: Listening on configured addresses
- **Session Creation**: Spawning new Session instances for clients
- **Client Registry**: Tracking named clients for session takeover
- **Resource Management**: Cleaning up disconnected sessions

Key data structures:
```rust
pub struct Broker {
    sessions: Mutex<HashMap<String, Arc<Session>>>,  // All sessions
    named_clients: DashMap<String, String>,          // clientId → sessionId
    router: Router,                                  // Shared router instance
}
```

### Session (`src/session.rs`)

Sessions represent individual client connections with a sophisticated state machine:

#### States
1. **WaitConnect**: Initial state, waiting for CONNECT packet
2. **Processing**: Normal operation, handling all packet types
3. **WaitTakeover**: Persistent session waiting for client reconnection
4. **Cleanup**: Terminal state, releasing resources

#### Key Features
- **Half-connected tracking**: Sessions exist before CONNECT for proper cleanup
- **Stream deadlock prevention**: Handlers receive stream references
- **Keep-alive monitoring**: Automatic timeout detection
- **Will message handling**: Storage and automatic publishing
- **QoS=1 support**: Full "at least once" delivery with retransmission
- **Direct response pattern**: Handlers write directly to client for lower latency
- **Packet ID management**: Sequential generation with wrap-around (1-65535)
- **In-flight message tracking**: VecDeque for efficient retransmission queue
- **DUP flag handling**: Proper duplicate detection and prevention

```rust
pub struct Runtime {
    id: String,                                    // Unique session ID
    client_id: Option<String>,                    // MQTT client ID
    clean_session: bool,                          // Clean session flag
    keep_alive: u16,                              // Keep-alive seconds
    stream: Option<Box<dyn AsyncStream>>,         // Network stream
    message_tx: mpsc::Sender<Packet>,             // Outgoing messages  
    will_message: Option<WillMessage>,            // Last Will storage
    next_packet_id: u16,                          // For generating packet IDs
    qos1_queue: VecDeque<InflightMessage>,       // In-flight QoS=1 messages
}
```

### Router (`src/router.rs`)

The Router implements MQTT's publish/subscribe pattern with advanced features:

#### Data Structures
```rust
pub struct Router {
    data: RwLock<RouterInternal>,
    retained_message_limit: usize,
}

struct RouterInternal {
    filters: HashMap<String, HashMap<String, Mailbox>>,    // filter → (session_id → mailbox)
    sessions: HashMap<String, HashSet<String>>,            // session_id → filters
    retained_messages: HashMap<String, PublishPacket>,     // topic → retained message
}
```

#### Wildcard Support
- **Single-level wildcard (`+`)**: Matches exactly one topic level
- **Multi-level wildcard (`#`)**: Matches any number of levels (must be last)

#### Topic Validation
- **Topic Names**: Must not be empty, must not contain null characters
- **Topic Filters**: Same as names plus wildcard validation rules
- **Protocol Compliance**: Full MQTT v3.1.1 specification adherence

#### Topic Matching Algorithm
1. Exact match check
2. Pattern matching with wildcards using recursive algorithm
3. Efficient bidirectional mapping for fast subscribe/unsubscribe
4. Support for all MQTT wildcard combinations

### Transport Layer (`src/transport.rs`)

Abstracts different network protocols behind a common interface:

```rust
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {
    fn split(self: Box<Self>) -> (Box<dyn AsyncRead + Send + Unpin>, 
                                  Box<dyn AsyncWrite + Send + Unpin>);
}
```

Implementations:
- **TCP**: Standard MQTT over TCP (port 1883)
- **WebSocket**: MQTT over WebSocket (planned)
- **TLS**: Secure MQTT (planned)

### Protocol Layer (`src/protocol/`)

Implements MQTT v3.1.1 packet encoding/decoding:

#### Packet Types
- Control packets: CONNECT, CONNACK, DISCONNECT, PINGREQ, PINGRESP
- Publish packets: PUBLISH, PUBACK, PUBREC, PUBREL, PUBCOMP
- Subscribe packets: SUBSCRIBE, SUBACK, UNSUBSCRIBE, UNSUBACK

#### Key Features
- Zero-copy parsing where possible using `bytes::Bytes`
- Streaming decoder for handling partial packets
- Comprehensive error handling
- Full protocol compliance

## Message Flow

### QoS=1 Message Flow
```
Publisher       Session         Router          Subscriber      Session
  |               |               |                 |              |
  |-- PUBLISH --> |               |                 |              |
  |               |-- Route ----> |                 |              |
  |               |               |-- Deliver ----> |              |
  |               |               |                 |-- PUBLISH -->|
  |<-- PUBACK --- |               |                 |              |
  |               |               |                 |<-- PUBACK ---|
```

### QoS=1 Retransmission Flow
```
Publisher       Session         Timer           Subscriber
  |               |               |                 |
  |-- PUBLISH --> |               |                 |
  |               |-- Store ----> |                 |
  |               |               |                 |-- PUBLISH -->
  |               |               |                 |  (no PUBACK)
  |               |<-- Timeout -- |                 |
  |               |               |                 |-- PUBLISH -->
  |               |               |                 |    (DUP=1)
  |<-- PUBACK --- |<---------------------------------- PUBACK ---|
  |               |-- Remove ---> |                 |
```

### Connection Establishment
```
Client          Session         Broker          Router
  |                |               |               |
  |-- TCP SYN ---->|               |               |
  |                |<-- Create ----|               |
  |-- CONNECT ---->|               |               |
  |                |-- Register -->|               |
  |<-- CONNACK ----|               |               |
```

### Message Publishing
```
Publisher       Session         Router          Subscribers
  |               |               |                 |
  |-- PUBLISH --> |               |                 |
  |               |-- Route ----> |                 |
  |               |               |-- Deliver ----> |
  |               |               |                 |-- PUBLISH -->
```

### Session Takeover
```
Client2         Session2        Broker         Session1        Client1
  |               |               |               |               |
  |-- CONNECT --> |               |               |               |
  |               |-- Collision ->|               |               |
  |               |               |-- Takeover -> |               |
  |               |               |               |-- DISCONNECT->|
  |<-- CONNACK ---|               |               |               |
```

## Concurrency Model

### Lock Hierarchy
To prevent deadlocks, locks are acquired in a consistent order:
1. Broker.sessions
2. Router.subscriptions
3. Router.retained_messages
4. Session internal locks

### Async Task Management
- Each session runs in its own Tokio task
- Graceful cancellation via CancellationToken
- Proper cleanup on task termination

### Message Passing
- MPSC channels for session mailboxes (router → session)
- Direct response pattern for control packets (reduced overhead)
- Bounded channels to prevent memory exhaustion
- Non-blocking sends with overflow handling
- Multiple QoS=1 messages in-flight simultaneously

## Error Handling

### Connection Errors
- I/O errors trigger session cleanup
- Malformed packets close connections
- Protocol violations return specific error codes

### System Errors
- Resource exhaustion handled gracefully
- Panic isolation per session
- Comprehensive error logging

## Performance Optimizations

### Memory Efficiency
- Shared immutable data with Arc
- Zero-copy message routing
- Efficient topic matching algorithms
- Configurable buffer sizes
- Multiple concurrent in-flight QoS=1 messages
- VecDeque for O(1) queue operations
- Efficient packet ID recycling (1-65535)
- Configurable retransmission intervals

### CPU Efficiency
- Lock-free operations where possible
- Batch processing for bulk operations
- Lazy evaluation of expensive operations

### Network Efficiency
- TCP_NODELAY for low latency
- Configurable write buffer sizes
- Efficient packet encoding/decoding

## Security Considerations

### Current Implementation
- Basic protocol validation
- Resource limits (connection count, message size)
- Clean session isolation

### Future Enhancements
- TLS/SSL support
- Authentication plugins
- Authorization framework
- Rate limiting

## Testing Strategy

### Unit Tests
- Packet encoding/decoding
- Router wildcard matching and topic validation
- Component isolation tests

### Integration Tests  
- Full protocol flows
- Session lifecycle scenarios
- QoS=1 delivery guarantees
- Message retransmission
- Duplicate handling
- Will message delivery
- Protocol compliance validation

### QoS=1 Specific Tests (10+ tests)
- Basic PUBLISH/PUBACK flow validation
- Message retransmission with DUP flag on timeout
- Duplicate detection preventing re-routing
- Maximum retry limit enforcement
- Multiple concurrent publishers/subscribers
- QoS downgrade scenarios (min of publish/subscribe QoS)
- In-flight message tracking and cleanup
- Packet ID assignment and acknowledgment
- Message ordering with multiple in-flight
- Performance under high QoS=1 load

### Performance Tests
- Throughput benchmarks
- Memory usage per connection
- QoS=1 vs QoS=0 performance

## Configuration

### Runtime Configuration
```toml
[server]
address = "0.0.0.0:1883"
retained_message_limit = 10000
max_retransmission_limit = 10
retransmission_interval_ms = 5000  # 5 seconds
```

### Compile-time Configuration
- Feature flags for optional components
- Platform-specific optimizations
- Debug vs release builds

## Future Architecture Considerations

### Horizontal Scaling
- Cluster mode with session migration
- Distributed routing table
- Load balancing strategies

### Persistence Layer
- Pluggable storage backends
- Session state persistence
- Message queue durability

### Monitoring & Observability
- Prometheus metrics
- Distributed tracing
- Health check endpoints

## Conclusion

IoTD's architecture provides a solid foundation for a high-performance MQTT server. The modular design, strong concurrency guarantees, and comprehensive testing ensure reliability and maintainability. The event-driven approach with Tokio enables efficient resource utilization and excellent scalability characteristics.