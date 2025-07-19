# IoTHub - High-Performance MQTT Server

A high-performance MQTT server implementation in Rust using Tokio, designed for scalability, reliability, and extensibility. Built with a modern async architecture supporting multiple transport protocols and thousands of concurrent connections.

## Features

### Current (Milestone 1) 🔄
- **MQTT v3.1.1 protocol support** with all packet types (CONNECT, CONNACK, PUBLISH, SUBSCRIBE, UNSUBSCRIBE, PINGREQ, PINGRESP, DISCONNECT)
- **Message routing system** with topic filtering and MQTT wildcard support (`+` single-level, `#` multi-level)
- **Event-driven architecture** with tokio::select! based packet handling
- **Race-condition-free shutdown** using CancellationToken across all components
- **Session management** with half-connected session tracking and proper cleanup
- **Transport abstraction** supporting multiple protocols (TCP, WebSocket, TLS)
- **Thread-safe operations** using RwLock and Arc for optimal concurrent access
- **UNIX signal handling** (SIGINT graceful shutdown, SIGTERM immediate exit)
- **Comprehensive testing** with 33 tests covering packet handling and routing

### Completed ✅
- Multi-broker architecture with Server → Broker → Session hierarchy
- Graceful shutdown with connection draining
- SessionId management with anonymous and client-based IDs
- Stream write lock deadlock prevention
- Comprehensive configuration system with TOML support

## Development Roadmap

### **Milestone 1**: Full MQTTv3 Server (QoS=0, no persistency/auth) 🔄
- ✅ Basic working architecture
- ✅ CONNECT, CONNACK, PUBLISH packet handling tested
- ✅ All packet types tested (SUBSCRIBE, UNSUBSCRIBE, PINGREQ, DISCONNECT)
- ✅ **Recently completed**: Message routing system with MQTT wildcard support (`+`, `#`)
- ❌ Clean session logic
- ❌ Retained messages
- ❌ Will messages
- ❌ Keep-alive mechanism

### **Milestone 2**: QoS=1 Support (in-memory)
- QoS=1 message acknowledgment
- Message persistence in memory
- Duplicate message handling

### **Milestone 3**: Basic Persistency & QoS=2
- Persistent storage interface
- QoS=2 message handling
- Session state persistence

### **Milestone 4**: Basic Authentication
- Config file-based authentication
- User credentials management
- Connection authentication

### **Milestone 5**: Enhanced Transport Layer
- TLS/SSL support
- WebSocket transport
- Transport layer security

### **Milestone 6**: Pluggable Architecture
- Pluggable persistence backends
- Pluggable authentication providers
- Pluggable authorization systems

### **Milestone 7**: Production Ready
- Enhanced logging and metrics
- Comprehensive documentation
- Usage examples and tutorials
- Single-node MQTT server for production use

## Architecture

```
iothub/
├── src/
│   ├── protocol/          # MQTT protocol implementation
│   ├── server.rs          # Core server orchestration
│   ├── broker.rs          # Connection broker per transport
│   ├── session.rs         # Session management
│   ├── router.rs          # Message routing
│   ├── transport.rs       # Transport abstraction
│   ├── config.rs          # Configuration management
│   ├── storage/           # Persistence layer (Milestone 3+)
│   └── auth/              # Authentication/Authorization (Milestone 4+)
├── docs/                  # Architecture and roadmap documentation
├── tests/                 # Integration tests
├── benches/              # Performance benchmarks
└── docker/               # Docker configuration
```

### Current Architecture

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

### Key Design Decisions

- **CancellationToken**: Used throughout for race-condition-free shutdown
- **Half-connected sessions**: Tracked separately until CONNECT received
- **Stream passing**: Packet handlers receive stream reference to avoid deadlocks
- **Thread-safe cleanup**: Lock-based swap pattern for safe concurrent operations
- **Event-driven**: tokio::select! for responsive packet and shutdown handling

## Quick Start

### Prerequisites
- Rust 1.75 or later
- Tokio runtime

### Building
```bash
cargo build --release
```

### Running
```bash
cargo run
# or
./target/release/iothub
```

The server will start on `localhost:1883` by default.

### Testing
```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_simple_connect
```

### Docker
```bash
# Build image
docker build -f docker/Dockerfile -t iothub .

# Run container
docker run -p 1883:1883 iothub
```

## Performance

Optimized for:
- **Low latency**: Async I/O with Tokio
- **Low memory**: Efficient data structures (DashMap, bytes)
- **High throughput**: Lock-free concurrent operations
- **Single binary**: No external dependencies in runtime

## MQTT Client Testing

You can test with any MQTT client:

```bash
# Using mosquitto clients
mosquitto_sub -h localhost -t "test/topic"
mosquitto_pub -h localhost -t "test/topic" -m "hello world"

# Using mqttx cli
mqttx sub -h localhost -t "test/topic"
mqttx pub -h localhost -t "test/topic" -m "hello world"
```

## Configuration

The server uses a comprehensive configuration system with TOML support:

```toml
[server]
listen_addresses = ["tcp://0.0.0.0:1883", "ws://0.0.0.0:9001"]
max_connections = 10000
session_timeout_secs = 300
keep_alive_timeout_secs = 60
max_packet_size = 1048576
retained_message_limit = 10000

[auth]
enabled = false
backend = "none"

[storage]
backend = "memory"

[logging]
level = "info"
format = "text"
```

Configuration can be provided through:
- Configuration files (TOML)
- Environment variables
- Command-line arguments
- Default values

## License

MIT