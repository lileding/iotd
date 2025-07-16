# IoTHub - High-Performance MQTT Server

A high-performance MQTT server implementation in Rust using Tokio, designed for scalability, reliability, and extensibility. Built with a modern async architecture supporting multiple transport protocols and thousands of concurrent connections.

## Features

### Stone 2 (Current) 🔄
- **Multi-broker architecture** with Server → Broker → Session hierarchy
- **Graceful shutdown** with connection draining
- **Session management** with unique sessionId and clientId conflict resolution
- **Transport abstraction** supporting multiple protocols (TCP, WebSocket, TLS)
- **Thread-safe operations** using DashMap and Arc for concurrent access
- **Comprehensive configuration** system with TOML support

### Stone 1 (Completed) ✅
- Basic pub/sub messaging with QoS level 0
- MQTT v3.1.1 protocol support
- Topic wildcards (+ and #)
- Retained messages
- Clean session support
- Basic TCP broker functionality

### Roadmap
- **Stone 3**: QoS level 1 and enhanced retained messages
- **Stone 4**: QoS level 2 support
- **Stone 5**: Authentication (no authorization)
- **Stone 6**: Persistence interface with plug-in storage
- **Stone 7**: Authorization support
- **Stone 8**: MQTT v5.0 support
- **Stone 9**: Multi-protocol support (WebSocket, TLS)
- **Stone 10**: Enterprise features (clustering, high availability)

## Architecture

```
iothub/
├── src/
│   ├── protocol/          # MQTT protocol implementation
│   ├── server.rs          # Core server orchestration
│   ├── session.rs         # Session management
│   ├── router.rs          # Message routing
│   ├── transport.rs       # Transport abstraction
│   ├── config.rs          # Configuration management
│   ├── storage/           # Persistence layer (Stone 6)
│   └── auth/              # Authentication/Authorization (Stone 5-7)
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
│  │              Sessions (by clientId)                     │
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