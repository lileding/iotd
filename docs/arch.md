# IoTHub Architecture

## Overview

IoTHub is a high-performance MQTT server implemented in Rust using Tokio for asynchronous I/O. The architecture features a Server → Broker → Session hierarchy designed for scalability, reliability, and extensibility with event-driven design and race-condition-free shutdown using CancellationToken.

## Core Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Server                               │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │   Broker    │  │   Broker    │  │   Broker    │        │
│  │  (TCP)      │  │  (WebSocket)│  │  (TLS)      │        │
│  └─────────────┘  └─────────────┘  └─────────────┘        │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┤
│  │              Sessions (by sessionId)                    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │  │  Session    │  │  Session    │  │  Session    │    │
│  │  │ (sessionId) │  │ (sessionId) │  │ (sessionId) │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │
│  └─────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────────┤
│  │                    Router                               │
│  │           (Routes by sessionId internally)              │
│  └─────────────────────────────────────────────────────────┤
│                                                             │
│  Config • Transport • Shutdown Management                  │
└─────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. Server

The `Server` is the central orchestrator that manages all components and provides the main API for the MQTT broker.

```rust
pub struct Server {
    config: Config,
    brokers: DashMap<String, Arc<Broker>>,
    sessions: DashMap<String, Arc<Session>>, // sessionId -> Session
    router: Router,
    draining: RwLock<bool>,
    shutdown_token: CancellationToken,
    completed_token: CancellationToken,
}
```

**Responsibilities:**
- Manages broker lifecycle and spawning
- Handles session registration and conflict resolution
- Provides unified API for session operations
- Coordinates graceful shutdown with CancellationToken
- Manages cross-cutting concerns (auth, metrics, logging)

**Key Methods:**
- `new(config: &Config) -> Arc<Self>` - Creates new server instance
- `run(self: Arc<Self>)` - Starts all brokers and waits for shutdown
- `shutdown(&self)` - Initiates graceful shutdown with drain mode
- `register_session(session: Arc<Session>) -> Result<()>` - Registers new session with conflict resolution
- `unregister_session(session_id: &str)` - Removes session on disconnect (thread-safe)

### 2. Broker

A `Broker` represents a network listener for a specific protocol and address.

```rust
pub struct Broker {
    bind_address: String,
    server: Arc<Server>,
    shutdown_token: CancellationToken,
    completed_token: CancellationToken,
    half_connected_sessions: Mutex<HashMap<String, Arc<Session>>>,
}
```

**Responsibilities:**
- Listens for incoming connections on specific transport
- Accepts new connections and creates sessions
- Tracks half-connected sessions until CONNECT received
- Handles protocol-specific connection setup
- Responds to shutdown signals via CancellationToken

**Supported Protocols:**
- `tcp://` - Plain TCP connections (implemented)
- `tcp+tls://` - TLS-encrypted TCP (future)
- `ws://` - WebSocket connections (future)
- `wss://` - Secure WebSocket connections (future)
- `unix://` - Unix domain sockets (future)

**Lifecycle:**
1. Initialize listener for protocol/address
2. Spawn async task with event loop
3. Select between accept() and shutdown signal
4. On accept: create new session, track as half-connected, spawn task
5. On shutdown: close listener, cleanup half-connected sessions, exit

### 3. Session

A `Session` represents a connected IoT client and manages its entire lifecycle.

```rust
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
```

**Responsibilities:**
- Manages sessionId lifecycle: `__anon_$uuid` → `__client_$clientId`
- Handles MQTT protocol packets with stream deadlock prevention
- Manages session state and client connection
- Handles session cleanup on disconnect
- Registers/unregisters with server for session management

**Lifecycle:**
1. **Creation**: Generate anonymous sessionId and track as half-connected
2. **Connection**: Process CONNECT packet, update sessionId if clientId provided
3. **Registration**: Register with server after CONNACK sent
4. **Message Processing**: Enter main event loop with tokio::select!
5. **Cleanup**: Remove from half-connected (if needed), unregister from server, close stream

**Event Loop:**
```rust
loop {
    tokio::select! {
        // Handle incoming MQTT packets
        result = self.handle_client_packet() => {
            match result {
                Ok(should_continue) => if !should_continue { break; },
                Err(e) => { error!("Session error: {}", e); break; }
            }
        }
        // Handle subscription messages from router
        Some(message) = message_rx.recv() => {
            if let Err(e) = self.handle_message(message).await {
                error!("Failed to handle message: {}", e);
                break; // Terminate on message delivery failure
            }
        }
        // Handle shutdown signal
        _ = self.shutdown_token.cancelled() => {
            info!("Session shutting down");
            break;
        }
    }
}
```

**Key Design Features:**
- **Stream passing**: Packet handlers receive stream reference to prevent deadlocks
- **Atomic connected flag**: Uses `AtomicBool` for performance
- **Half-connected tracking**: Removed only if connection never completed
- **Error handling**: Message delivery failures terminate session

### 4. Router

The `Router` handles message routing and subscription management.

```rust
pub struct Router {
    // Internal routing uses sessionId for session identification
    // External MQTT compatibility uses clientId for session lookup
}
```

**Responsibilities:**
- Uses sessionId internally for routing (avoids client ID conflicts)
- Manages topic subscriptions and unsubscriptions
- Routes published messages to matching subscribers
- Handles wildcard topic matching (`+` and `#`)
- Maintains thread-safe subscription state
- Provides session cleanup on disconnect

**Topic Matching:**
- `+` - Single-level wildcard (e.g., `sensor/+/temperature`)
- `#` - Multi-level wildcard (e.g., `sensor/#`)
- Exact match for non-wildcard topics

**Future Optimizations:**
- Implement topic trie for O(log n) matching
- Add subscription statistics and monitoring
- Optimize wildcard matching performance

### 5. Configuration

```rust
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
}

pub struct ServerConfig {
    pub listen_addresses: Vec<String>,
    pub max_connections: usize,
    pub session_timeout_secs: u64,
    pub keep_alive_timeout_secs: u64,
    pub max_packet_size: usize,
    pub retained_message_limit: usize,
}
```

**Configuration Sources:**
- Configuration files (TOML/JSON)
- Environment variables
- Command-line arguments
- Default values

## Transport Abstraction

The architecture supports multiple transport protocols through traits:

```rust
#[async_trait]
pub trait AsyncListener: Send + Sync {
    async fn accept(&self) -> Result<Box<dyn AsyncStream>>;
    async fn local_addr(&self) -> Result<SocketAddr>;
    async fn close(&self) -> Result<()>;
}

#[async_trait]
pub trait AsyncStream: Send + Sync {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    async fn write(&mut self, buf: &[u8]) -> Result<usize>;
    async fn flush(&mut self) -> Result<()>;
    async fn close(&mut self) -> Result<()>;
}
```

## Concurrency Model

### Thread Safety
- **Server**: Shared via `Arc<Server>` across all components
- **Sessions**: Stored in `DashMap<String, Arc<Session>>` (sessionId -> Session)
- **Brokers**: Stored in `DashMap<String, Arc<Broker>>` (address -> Broker)
- **Router**: Thread-safe operations for session management
- **Shutdown**: Coordinated via `CancellationToken` for race-condition-free shutdown

### Async Tasks
- **Server**: Main coordinator, spawns broker tasks
- **Broker**: One task per protocol/address, spawns session tasks
- **Session**: One task per client connection
- **Router**: Embedded in server, no separate task

### Message Passing
- **Shutdown Signals**: `CancellationToken` for graceful shutdown coordination
- **Session Management**: Direct async method calls between components
- **Inter-component Communication**: Server as central message hub

## Key Design Decisions

### 1. CancellationToken Architecture
- **Problem**: `Notify` had race conditions where shutdown could be missed
- **Solution**: `CancellationToken` maintains state, eliminates race conditions
- **Implementation**: Used throughout Server, Broker, Session for consistent shutdown

### 2. Half-connected Session Tracking
- **Problem**: Sessions need cleanup even if CONNECT never received
- **Solution**: Track sessions in broker until CONNECT, then move to server
- **Implementation**: Mutex-protected HashMap with lock-based swap for cleanup

### 3. Stream Deadlock Prevention
- **Problem**: Packet handlers acquiring stream lock while already held
- **Solution**: Pass stream reference to packet handlers
- **Implementation**: Handlers accept `&mut dyn AsyncStream` parameter

### 4. SessionId Management
- **Problem**: Client ID conflicts and routing complexity
- **Solution**: Internal sessionId separate from MQTT clientId
- **Implementation**: `__anon_$uuid` → `__client_$clientId` on CONNECT

### 5. Thread-safe Cleanup
- **Problem**: Concurrent access to session collections during shutdown
- **Solution**: Lock-based swap pattern with DashMap
- **Implementation**: Clone map, clear original, iterate over clone

## Development Status (Milestone 1)

### ✅ Completed
- Event-driven architecture with tokio::select!
- Race-condition-free shutdown using CancellationToken
- Half-connected session tracking and cleanup
- Stream deadlock prevention in packet handlers
- UNIX signal handling (SIGINT graceful, SIGTERM immediate)
- Basic MQTT v3.1.1 packet handling (CONNECT, CONNACK, PUBLISH tested)
- Session lifecycle management with proper cleanup

### 🔄 In Progress
- Testing all packet types (SUBSCRIBE, UNSUBSCRIBE, PINGREQ, DISCONNECT)

### ❌ Missing for Milestone 1
- Message routing system (biggest gap)
- Clean session logic
- Retained messages
- Will messages
- Keep-alive mechanism

## Performance Characteristics

### Scalability
- **Horizontal**: Multiple brokers per server
- **Vertical**: Lock-free concurrent data structures
- **Memory**: Efficient message routing without copying
- **CPU**: Minimal context switching with async/await

### Benchmarks (Target)
- **Connections**: 10,000+ concurrent clients
- **Throughput**: 100,000+ messages/second
- **Latency**: Sub-millisecond message routing
- **Memory**: <1MB per 1,000 idle connections

## Security Model

### Authentication
- Pluggable authentication backends
- Support for username/password, certificates, tokens
- Future: OAuth2, JWT, custom authentication

### Authorization
- Topic-based access control
- Per-client permissions
- Wildcard permission matching

### Transport Security
- TLS support for all protocols
- Certificate-based authentication
- Secure WebSocket (WSS) support

## Monitoring and Observability

### Metrics
- Connection count and rate
- Message throughput and latency
- Topic subscription statistics
- Error rates and types

### Logging
- Structured logging with tracing
- Configurable log levels
- JSON output for log aggregation

### Health Checks
- Server health endpoints
- Broker status monitoring
- Session health tracking

## Extension Points

### Plugin Architecture
- **Authentication**: Custom auth backends
- **Authorization**: Custom ACL providers
- **Storage**: Pluggable persistence layers
- **Metrics**: Custom metrics collectors
- **Hooks**: Pre/post message processing

### Custom Protocols
- Protocol-specific packet handling
- Custom transport implementations
- Protocol version negotiation

This architecture provides a solid foundation for building a production-ready MQTT broker that can scale to handle thousands of concurrent connections while maintaining high performance and reliability through careful concurrency design and race-condition-free shutdown mechanisms.