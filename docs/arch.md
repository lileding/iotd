# IoTHub Architecture

## Overview

IoTHub is a high-performance MQTT server implemented in Rust using Tokio for asynchronous I/O. The architecture features a Server → Broker → Session hierarchy designed for scalability, reliability, and extensibility. The current implementation (Stone 2) introduces multi-broker support, graceful shutdown, session management with unique sessionId, and transport abstraction for future protocol support.

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
│  │                   Sessions                              │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │  │  Session    │  │  Session    │  │  Session    │    │
│  │  │ (Client A)  │  │ (Client B)  │  │ (Client C)  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │
│  └─────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────────┤
│  │                    Router                               │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │  │Topic Filter │  │Topic Filter │  │Topic Filter │    │
│  │  │ → Sessions  │  │ → Sessions  │  │ → Sessions  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘    │
│  └─────────────────────────────────────────────────────────┤
│                                                             │
│  Config • Auth • Storage • Metrics                         │
└─────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. Server

The `Server` is the central orchestrator that manages all components and provides the main API for the MQTT broker.

```rust
pub struct Server {
    config: Config,
    brokers: DashMap<String, Arc<Broker>>,
    sessions: Mutex<DashMap<String, Arc<Session>>>, // clientId -> Session
    router: Router,
    draining: RwLock<bool>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}
```

**Responsibilities:**
- Manages broker lifecycle and spawning
- Handles session registration and conflict resolution
- Provides unified API for session operations
- Coordinates graceful shutdown
- Manages cross-cutting concerns (auth, metrics, logging)

**Key Methods:**
- `new(config: &Config) -> Arc<Self>` - Creates new server instance
- `run(self: Arc<Self>) -> Result<()>` - Starts all brokers and waits for shutdown
- `shutdown(&self)` - Initiates graceful shutdown with drain mode
- `register_session(session: Arc<Session>) -> Result<()>` - Registers new session with conflict resolution
- `unregister_session(session_id: &str)` - Removes session on disconnect

### 2. Broker

A `Broker` represents a network listener for a specific protocol and address.

```rust
struct Broker {
    bind_address: String,
    server: Arc<Server>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}
```

**Responsibilities:**
- Listens for incoming connections on specific transport
- Accepts new connections and creates sessions
- Handles protocol-specific connection setup
- Responds to shutdown signals

**Supported Protocols:**
- `tcp://` - Plain TCP connections (implemented)
- `tcp+tls://` - TLS-encrypted TCP (future)
- `ws://` - WebSocket connections (future)
- `wss://` - Secure WebSocket connections (future)
- `unix://` - Unix domain sockets (future)

**Lifecycle:**
1. Initialize listener for protocol/address
2. Spawn async task with infinite loop
3. Select between accept() and shutdown signal
4. On accept: create new session and spawn task
5. On shutdown: close listener and exit loop

### 3. Session

A `Session` represents a connected IoT client and manages its entire lifecycle.

```rust
pub struct Session {
    session_id: String,
    client_id: String,
    clean_session: bool,
    keep_alive: u16,
    stream: RwLock<Box<dyn AsyncStream>>,
    server: Arc<Server>,
    shutdown: Arc<Notify>,
    completed: Arc<Notify>,
}
```

**Responsibilities:**
- Generates unique sessionId for internal routing
- Handles MQTT protocol packets (CONNECT, PUBLISH, SUBSCRIBE, etc.)
- Manages session state and client connection
- Provides default client ID (`"__iothub_{sessionId}"`) if not provided
- Handles session cleanup on disconnect
- Registers/unregisters with server for session management

**Lifecycle:**
1. **Connection**: Process CONNECT packet
2. **Authentication**: Validate client credentials
3. **Registration**: Register with server (handles client ID conflicts)
4. **Authorization**: Check topic permissions
5. **Message Processing**: Enter main event loop
6. **Cleanup**: Unregister subscriptions and close connection

**Event Loop:**
```rust
loop {
    tokio::select! {
        // Handle incoming MQTT packets
        packet = self.read_packet() => {
            self.process_packet(packet?).await?;
        }
        // Handle subscription messages from router
        msg = self.subscription_rx.recv() => {
            self.send_message(msg?).await?;
        }
        // Handle shutdown signal
        _ = self.shutdown_rx.recv() => break,
    }
}
```

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
- **Sessions**: Stored in `Mutex<DashMap<String, Arc<Session>>>` (clientId -> Session)
- **Brokers**: Stored in `DashMap<String, Arc<Broker>>` (address -> Broker)
- **Router**: Thread-safe operations for session management
- **Shutdown**: Coordinated via `Arc<Notify>` for graceful shutdown

### Async Tasks
- **Server**: Main coordinator, spawns broker tasks
- **Broker**: One task per protocol/address, spawns session tasks
- **Session**: One task per client connection
- **Router**: Embedded in server, no separate task

### Message Passing
- **Shutdown Signals**: `Arc<Notify>` for graceful shutdown coordination
- **Session Management**: Direct async method calls between components
- **Inter-component Communication**: Server as central message hub

## Error Handling

```rust
#[derive(Error, Debug)]
pub enum ServerError {
    #[error("Broker error: {0}")]
    Broker(#[from] BrokerError),
    #[error("Session error: {0}")]
    Session(#[from] SessionError),
    #[error("Router error: {0}")]
    Router(#[from] RouterError),
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("Authentication error: {0}")]
    Auth(#[from] AuthError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
}
```

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

This architecture provides a solid foundation for building a production-ready MQTT broker that can scale to handle thousands of concurrent connections while maintaining high performance and reliability.