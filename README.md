# IoTD - High-Performance MQTT Daemon

A high-performance MQTT server daemon implementation in Rust using Tokio, designed for scalability, reliability, and extensibility. Built with a modern async architecture supporting multiple transport protocols and thousands of concurrent connections.

## Features

- **MQTT v3.1.1 protocol support** with all packet types
- **Message routing** with full MQTT wildcard support (`+` single-level, `#` multi-level)
- **Clean session** with session takeover and proper cleanup
- **Keep-alive mechanism** with configurable timeouts
- **Retained messages** with storage limits and wildcard delivery
- **Will messages** (Last Will and Testament) support
- **Event-driven architecture** using tokio::select! for high performance
- **Race-condition-free shutdown** using CancellationToken
- **Multi-broker architecture** supporting multiple transport protocols
- **Thread-safe operations** with optimal concurrent access patterns
- **UNIX signal handling** (SIGINT graceful, SIGTERM immediate)
- **Comprehensive configuration** with TOML support
- **Extensive test coverage** with 67 tests validating all functionality

## Current Status

IoTD is currently in **Milestone 1** development, implementing a full MQTT v3.1.1 server with QoS=0 support.

### Completed Features ✅
- **Complete MQTT v3.1.1 protocol support** with all packet types
- **Message routing system** with full MQTT wildcard support (`+`, `#`)
- **Clean session logic** with session takeover and DISCONNECT notifications
- **Keep-alive mechanism** with configurable timeouts and automatic cleanup
- **Retained messages** with storage limits and wildcard delivery
- **Will messages** (Last Will and Testament) support
- **Race-condition-free architecture** using CancellationToken
- **Comprehensive test suite** with 67 tests (31 packet unit tests, 10 router unit tests, 26 integration tests)

### In Progress 🔄
- Protocol compliance enhancements (error codes, client ID validation)

### Upcoming Features 📋
- QoS=1 and QoS=2 support
- Persistent storage backends
- Authentication and authorization
- TLS/SSL and WebSocket transports
- Production-ready features (metrics, monitoring, clustering)

For a detailed development roadmap, see [docs/roadmap.md](docs/roadmap.md).

## Architecture

IoTD follows a modular, event-driven architecture built on Tokio's async runtime:

- **Server → Broker → Session → Router** hierarchy for clean separation of concerns
- **Event-driven design** using `tokio::select!` for responsive packet handling
- **Race-condition-free** shutdown using `CancellationToken` throughout
- **Thread-safe operations** with optimal locking strategies
- **Zero-copy message routing** for maximum performance

Key components:
- `server.rs` - Lifecycle management and signal handling
- `broker.rs` - Connection acceptance and session management
- `session.rs` - Client state machine and packet processing
- `router.rs` - Publish/subscribe with wildcard support
- `protocol/` - MQTT v3.1.1 packet encoding/decoding

For detailed architecture documentation, see [docs/arch.md](docs/arch.md).

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
./target/release/iotd
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
docker build -t iotd .

# Run container
docker run -p 1883:1883 iotd
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