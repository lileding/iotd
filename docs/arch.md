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

```rust
pub struct Session {
    id: String,                                    // Unique session ID
    client_id: Mutex<Option<String>>,             // MQTT client ID
    clean_session: AtomicBool,                    // Clean session flag
    keep_alive: AtomicU16,                        // Keep-alive seconds
    stream: Mutex<Option<Box<dyn AsyncStream>>>,  // Network stream
    message_tx: mpsc::Sender<Packet>,             // Outgoing messages
    will_message: Mutex<Option<WillMessage>>,     // Last Will storage
}
```

### Router (`src/router.rs`)

The Router implements MQTT's publish/subscribe pattern with advanced features:

#### Data Structures
```rust
pub struct Router {
    subscriptions: Arc<RwLock<SubscriptionTree>>,
    retained_messages: Arc<RwLock<HashMap<String, RetainedMessage>>>,
    retained_limit: usize,
}

struct SubscriptionTree {
    topic_filters: HashMap<String, HashSet<String>>,  // filter → sessions
    session_filters: HashMap<String, HashSet<String>>, // session → filters
}
```

#### Wildcard Support
- **Single-level wildcard (`+`)**: Matches exactly one topic level
- **Multi-level wildcard (`#`)**: Matches any number of levels (must be last)

#### Topic Matching Algorithm
1. Exact match check
2. Pattern matching with wildcards
3. Special handling for `$SYS/` topics
4. Efficient trie-like traversal

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
- MPSC channels for session mailboxes
- Bounded channels to prevent memory exhaustion
- Non-blocking sends with overflow handling

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
- Packet encoding/decoding (31 tests)
- Router wildcard matching (10 tests)
- Component isolation tests

### Integration Tests
- Full protocol flows (26 tests)
- Session lifecycle scenarios
- Error handling paths
- Will message delivery

### Performance Tests
- Throughput benchmarks
- Latency measurements
- Concurrent connection limits

## Configuration

### Runtime Configuration
```toml
[server]
listen_addresses = ["tcp://0.0.0.0:1883"]
max_connections = 10000
max_packet_size = 1048576

[session]
keep_alive_timeout_secs = 90
session_timeout_secs = 300

[router]
retained_message_limit = 10000
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