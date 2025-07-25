# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## IMPORTANT: MQTT 3.1.1 Specification Compliance

**You MUST follow the MQTT 3.1.1 Specification exactly.**
**You should NOT design features not presented in the MQTT 3.1.1 Specification.**

When implementing MQTT features:
1. Always refer to the official MQTT 3.1.1 specification
2. Do not add "improvements" beyond what the spec requires
3. Keep implementations simple and spec-compliant
4. Avoid over-engineering based on assumptions

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
./target/release/iotd
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
docker build -t iotd .

# Run container
docker run -p 1883:1883 iotd
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

### Current Status (Milestone 2 - QoS=1 Support)

**✅ Milestone 1 Completed:**
- Event-driven architecture with tokio::select!
- Race-condition-free shutdown using CancellationToken
- Complete MQTT v3.1.1 packet handling (all packet types)
- Message routing system with topic filtering and wildcard support
- Clean session logic with session takeover
- Keep-alive mechanism
- Retained messages with wildcard delivery
- Will messages (Last Will and Testament)
- Comprehensive test suite (74+ tests)

**✅ Milestone 2 Progress:**
- QoS=1 message delivery with PUBACK acknowledgment
- Message retransmission with DUP flag
- In-flight message tracking
- Direct response pattern for reduced latency
- Message ordering guarantees (though not required by spec)

**📋 Milestone 2 Remaining:**
- Simplify implementation to match MQTT 3.1.1 spec exactly
- Remove over-engineered features (exponential backoff, ordered queue)
- Implement proper packet ID management
- Support multiple in-flight messages
- Session state recovery for clean_session=false

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
- **Session Takeover**: Clean session=false clients can take over existing sessions with DISCONNECT notification
- **Keep-Alive Monitoring**: Automatic disconnection of inactive clients based on keep-alive timeout

### Common Issues and Solutions

1. **Deadlock in packet handling**: Pass stream reference to handlers instead of acquiring lock again
2. **Race conditions in shutdown**: Use CancellationToken instead of Notify for state-maintaining cancellation
3. **Session cleanup ordering**: Half-connected removal before server registration
4. **Stream write failures**: Terminate session loop on message delivery errors