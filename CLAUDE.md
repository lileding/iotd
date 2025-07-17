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

IoTHub is a high-performance MQTT server implemented in Rust using Tokio. The architecture follows a **Server → Broker → Session** hierarchy:

### Core Components

- **Server** (`src/server.rs`): Central orchestrator managing brokers, sessions, routing, and shutdown
- **Broker** (`src/server.rs`): Protocol-specific network listeners (TCP, WebSocket, TLS planned)
- **Session** (`src/session.rs`): Individual client connection handlers with unique sessionId
- **Router** (`src/router.rs`): Message routing and subscription management using sessionId internally
- **Protocol** (`src/protocol/`): MQTT v3.1.1 packet parsing and handling
- **Transport** (`src/transport.rs`): Abstraction layer for different network protocols
- **Config** (`src/config.rs`): Comprehensive configuration system with TOML support

### Key Design Patterns

1. **Session Management**: Each session has a unique `sessionId` for internal routing, separate from MQTT `clientId` to handle conflicts
2. **Graceful Shutdown**: Coordinated via `Arc<Notify>` across all components with connection draining
3. **Thread Safety**: Uses `DashMap` and `Arc` for concurrent access to shared state
4. **Async Architecture**: Tokio-based with select! loops for handling multiple event sources
5. **Transport Abstraction**: Traits allow multiple protocols (TCP implemented, WebSocket/TLS planned)

### Project Structure
- `src/auth/` - Authentication and authorization (Stone 5-7)
- `src/protocol/` - MQTT protocol implementation (v3.1.1 complete, v5.0 planned)
- `src/storage/` - Persistence layer interfaces (Stone 6)
- `tests/` - Integration and unit tests
- `benches/` - Performance benchmarks
- `docs/` - Detailed architecture documentation

### Current Status (Stone 2)
- Multi-broker architecture with graceful shutdown
- Session management with clientId conflict resolution
- Transport abstraction supporting TCP (WebSocket/TLS planned)
- Thread-safe operations with DashMap and Arc

### Testing Strategy
- Integration tests connect to test server on port 18833
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