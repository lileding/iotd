# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Running
```bash
# Build the project
cargo build

# Build optimized release version
cargo build --release

# Run the server
cargo run

# Run from release binary
./target/release/iothub
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with output visible
cargo test -- --nocapture

# Run specific test
cargo test test_simple_connect

# Run integration tests specifically
cargo test --test integration_test
cargo test --test simple_test
```

### Benchmarking
```bash
# Run performance benchmarks
cargo bench

# Run specific benchmark
cargo bench broker_benchmark
```

### Docker
```bash
# Build Docker image
docker build -f docker/Dockerfile -t iothub .

# Run container
docker run -p 1883:1883 iothub
```

## Architecture Overview

IoTHub is a high-performance MQTT server implemented in Rust using Tokio. The architecture follows a **Server → Broker → Session** hierarchy with event-driven design and race-condition-free shutdown.

### Core Components

- **Server** (`src/server.rs`): Central orchestrator managing brokers, sessions, routing, and shutdown
- **Broker** (`src/broker.rs`): Protocol-specific network listeners (TCP, WebSocket, TLS planned)
- **Session** (`src/session.rs`): Individual client connection handlers with unique sessionId
- **Router** (`src/router.rs`): Message routing and subscription management using sessionId internally
- **Protocol** (`src/protocol/`): MQTT v3.1.1 packet parsing and handling
- **Transport** (`src/transport.rs`): Abstraction layer for different network protocols
- **Config** (`src/config.rs`): Comprehensive configuration system with TOML support

### Key Design Patterns

1. **CancellationToken Architecture**: Uses `tokio_util::sync::CancellationToken` throughout for race-condition-free shutdown
2. **Half-connected Sessions**: Tracked separately until CONNECT received to manage incomplete connections
3. **Stream Passing**: Packet handlers receive stream reference to avoid write lock deadlocks
4. **Thread-safe Cleanup**: Lock-based swap pattern for safe concurrent operations
5. **Event-driven**: tokio::select! for responsive packet and shutdown handling
6. **Session Management**: SessionId starts as `__anon_$uuid`, becomes `__client_$clientId` after CONNECT

### Current Status (Milestone 1)

**✅ Completed:**
- Event-driven architecture with tokio::select! 
- Race-condition-free shutdown using CancellationToken
- Half-connected session tracking and cleanup
- Stream deadlock prevention
- UNIX signal handling (SIGINT graceful, SIGTERM immediate)
- Complete MQTT v3.1.1 packet handling (all packet types tested)
- **Message routing system** with topic filtering and MQTT wildcard support (`+`, `#`)
- Comprehensive test suite (33 tests: 10 router unit tests, 23 integration/packet tests)

**❌ Missing for Milestone 1:**
- Clean session logic
- Retained messages
- Will messages
- Keep-alive mechanism

### Project Structure
- `src/auth/` - Authentication and authorization (Milestone 4+)
- `src/protocol/` - MQTT protocol implementation (v3.1.1 in progress)
- `src/storage/` - Persistence layer interfaces (Milestone 3+)
- `tests/` - Integration and unit tests
- `benches/` - Performance benchmarks
- `docs/` - Detailed architecture documentation

### Development Roadmap

**Milestone 1** (Current): Full MQTTv3 Server (QoS=0, no persistency/auth)
**Milestone 2**: QoS=1 Support (in-memory)
**Milestone 3**: Basic Persistency & QoS=2
**Milestone 4**: Basic Authentication
**Milestone 5**: Enhanced Transport Layer (TLS)
**Milestone 6**: Pluggable Architecture
**Milestone 7**: Production Ready

### Testing Strategy
- Integration tests connect to test server on different ports (18831, 18832, 18833)
- Tests verify MQTT protocol compliance (CONNECT/CONNACK flows)
- Simple connection tests validate basic server functionality
- Use `cargo test test_simple_connect` for basic connectivity verification

### Configuration
The server uses a comprehensive TOML-based configuration system with support for:
- Multiple listen addresses with protocol prefixes (`tcp://`, `ws://`, `tls://`)
- Connection limits and timeouts
- Authentication and storage backends
- Logging configuration

Default server runs on `127.0.0.1:1883` for MQTT clients.

### Key Technical Notes

- **Deadlock Prevention**: Session packet handlers accept stream parameter to avoid double-locking
- **Session Cleanup**: `cleanup_and_exit()` removes half-connected sessions only if not connected
- **Shutdown Sequence**: Server → Brokers → Sessions with proper ordering
- **Error Handling**: `handle_message` failures now terminate session loop
- **Import Pattern**: Use `use crate::protocol::packet;` for module-level imports
- **Router Architecture**: Uses RwLock with bidirectional mapping (filter→sessions, session→filters) for efficient routing and cleanup
- **Wildcard Matching**: Implements MQTT-compliant topic matching with `+` (single-level) and `#` (multi-level) wildcards

### Common Issues and Solutions

1. **Deadlock in packet handling**: Pass stream reference to handlers instead of acquiring lock again
2. **Race conditions in shutdown**: Use CancellationToken instead of Notify for state-maintaining cancellation
3. **Session cleanup ordering**: Half-connected removal before server registration
4. **Stream write failures**: Terminate session loop on message delivery errors