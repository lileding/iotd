# IoTD - High-Performance MQTT Daemon

```
    ___     _____     ____  
   |_ _|___/__   \   |  _ \ 
    | |/ _ \ / /\ /  | | | |
    | | (_) / /  \ \ | |_| |
   |___\___/\/    \_\|____/ 
                            
   IoT Daemon - MQTT Server
```

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
- **Extensive test coverage** with 74 tests validating all functionality

## Current Status

**IoTD has nearly completed Milestone 2 - QoS=1 Support! 🚀**

The project has implemented full QoS=1 "at least once" delivery guarantees. Only cleanup tasks remain before moving to Milestone 3 (Persistence).

### Completed Features ✅

#### Milestone 1 - Basic MQTT Server (QoS=0)
- **Complete MQTT v3.1.1 protocol support** with all packet types
- **Message routing system** with full MQTT wildcard support (`+`, `#`)
- **Clean session logic** with session takeover and DISCONNECT notifications
- **Keep-alive mechanism** with configurable timeouts and automatic cleanup
- **Retained messages** with storage limits and wildcard delivery
- **Will messages** (Last Will and Testament) support
- **Protocol compliance** with validation, error codes, and client ID rules
- **Topic validation** for both topic names and subscription filters
- **Race-condition-free architecture** using CancellationToken
- **Comprehensive test suite** with 74+ tests

#### Milestone 2 - QoS=1 Support (Nearly Complete)
- ✅ **QoS=1 message delivery** - "At least once" guarantee implemented
- ✅ **PUBACK handling** - Proper acknowledgment flow
- ✅ **Message retransmission** - Automatic retry with DUP flag
- ✅ **Multiple in-flight messages** - No artificial ordering constraints
- ✅ **Duplicate detection** - Prevents routing duplicates
- ✅ **Configurable retransmission** - Interval and retry limits
- ✅ **Comprehensive QoS=1 tests** - All edge cases covered
- 📋 **Packet ID management** - Basic implementation, needs collision detection
- 📋 **Session persistence** - For clean_session=false support

### Roadmap 📋

#### Near-term (v0.x - v1.0)
- **Milestone 2** (Current): Final QoS=1 cleanup tasks
- **Milestone 3**: Persistence layer (SQLite, file-based)
- **Milestone 4**: Security (TLS, authentication, ACLs)
- **Milestone 5**: QoS=2 "exactly once" delivery
- **Milestone 6**: Observability (Prometheus, Grafana)
- **Milestone 7**: Flow control & production features
- **v1.0**: Production-ready single-node broker

#### Long-term (v2.0+)
- **v2.0**: MQTT 5.0 protocol support
- **v3.0**: Clustering and high availability
- **v4.0**: Multi-tenancy and enterprise features

For a detailed development roadmap, see [docs/roadmap.md](docs/roadmap.md).

## Platform Support

IoTD has been manually tested and verified to work on the following platforms:

### Tested Platforms ✅
- **macOS (Apple Silicon)** - Native ARM64 support
- **Linux GNU (aarch64)** - ARM64 with glibc
- **Linux musl (aarch64)** - ARM64 with musl libc (Alpine Linux)
- **Linux GNU (x86_64)** - Intel/AMD 64-bit with glibc
- **Linux musl (x86_64)** - Intel/AMD 64-bit with musl libc (Alpine Linux)

### Network Support
- **IPv4** - Full support on all platforms
- **IPv6** - Full support on all platforms
- **Dual-stack** - Can listen on both IPv4 and IPv6 simultaneously

The single binary design and Rust's cross-platform capabilities ensure consistent behavior across all supported platforms.

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
# Default (localhost only)
cargo run
# or
./target/release/iotd

# Custom listen address
./target/release/iotd -l 0.0.0.0:1883    # All IPv4 interfaces
./target/release/iotd -l [::]:1883       # All IPv6 interfaces
./target/release/iotd -l 127.0.0.1:8883  # Custom port

# Help and version
./target/release/iotd --help
./target/release/iotd --version
```

The server listens on `127.0.0.1:1883` by default (localhost only).

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
- **High throughput**: Multiple in-flight QoS=1 messages
- **Memory efficient**: ~8.45 KB per connection
- **Single binary**: No external dependencies
- **MQTT spec compliant**: Strict adherence to MQTT 3.1.1

### Benchmark Results

Tested on Mac Studio 2025 (M4 Max 16-core) with `RUST_LOG=error`:

- **Memory efficiency**: Only 8.45 KB per connection
- **1000 connections**: ~10 MB total memory usage
- **Throughput**: 8,600+ messages/second routed to 950 subscribers
- **Message rate**: 4,000-9,000 msg/sec for small messages

For detailed benchmarks and testing tools, see [BENCHMARKS.md](BENCHMARKS.md).

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