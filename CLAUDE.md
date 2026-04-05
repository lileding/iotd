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
cargo build                    # Debug build
cargo build --release          # Optimized release build
cargo run                      # Run server (default: 127.0.0.1:1883)
cargo run -- -c config.toml    # Run with config file
cargo run -- -l 0.0.0.0:1883   # Run with custom listen address
./target/release/iotd --version # Show version with git revision
```

### Testing
```bash
cargo test                          # Run all tests (136 tests)
cargo test -- --nocapture           # Run tests with stdout visible
cargo test test_simple_connect      # Run a specific test by name
cargo test --test integration_test  # Run integration tests only
cargo test --test qos1_test         # Run QoS=1 tests only
cargo test --test qos2_test         # Run QoS=2 tests only
cargo test --test tls_test          # Run TLS tests only
cargo test --test packet_test       # Run packet encoding/decoding tests
cargo test --test simple_test       # Run basic connectivity tests
```

### Linting and Formatting
```bash
cargo fmt                  # Format code
cargo fmt -- --check       # Check formatting (CI uses this)
cargo clippy -- -D warnings # Run clippy with warnings as errors
make check                 # Run fmt-check + clippy + test together
make fix                   # Auto-format then run clippy
```

### Benchmarking
```bash
cargo bench                    # Run all benchmarks
cargo bench broker_benchmark   # Run specific benchmark
cargo bench performance_benchmark
./run_benchmarks.sh            # Run benchmarks via script
```

### Docker
```bash
docker build -t iotd .
docker run -p 1883:1883 iotd
```

## Architecture Overview

iotd is a high-performance MQTT v3.1.1 server implemented in Rust using Tokio. The architecture follows a **Server -> Broker -> Session** hierarchy with event-driven design and race-condition-free shutdown.

### Source File Layout (~6,600 lines of Rust)

```
src/
  main.rs          (123 lines)  - CLI entry point, arg parsing, signal handling
  lib.rs           (9 lines)    - Module declarations
  server.rs        (332 lines)  - Central orchestrator: broker management, routing, shutdown
  broker.rs        (145 lines)  - Network listeners (TCP, TLS)
  session.rs       (1368 lines) - Client connection handler, QoS state machines, keep-alive
  router.rs        (849 lines)  - Pub/sub routing, topic filter matching, wildcards
  transport.rs     (289 lines)  - AsyncStream trait abstraction over TCP/TLS
  config.rs        (305 lines)  - TOML config with serde, defaults, validation
  protocol/
    mod.rs         (4 lines)    - Module re-exports
    packet.rs      (1186 lines) - MQTT packet types, encoding/decoding, all 14 packet types
    v3.rs          (20 lines)   - MQTT v3.1.1 protocol constants
  auth/
    mod.rs         (30 lines)   - Auth module, factory functions
    traits.rs      (39 lines)   - Authenticator and Authorizer traits
    allow_all.rs   (72 lines)   - Default allow-all backend
    password_file.rs (155 lines) - File-based username/password auth
    acl_file.rs    (214 lines)  - File-based topic ACL rules
  storage/
    mod.rs         (20 lines)   - Storage module, factory functions
    traits.rs      (83 lines)   - Storage trait definition (async_trait)
    types.rs       (129 lines)  - Persisted types (session, subscription, inflight, QoS2)
    memory.rs      (309 lines)  - In-memory storage backend (HashMap-based)
    sqlite.rs      (826 lines)  - SQLite storage backend (rusqlite)
tests/
  simple_test.rs        (53 lines)   - Basic connectivity
  packet_test.rs        (681 lines)  - Packet encode/decode
  integration_test.rs   (1404 lines) - Full MQTT protocol flows
  qos1_test.rs          (841 lines)  - QoS=1 delivery tests
  qos2_test.rs          (408 lines)  - QoS=2 delivery tests
  tls_test.rs           (297 lines)  - TLS connection tests
benches/
  broker_benchmark.rs   - Broker throughput benchmarks
  performance_benchmark.rs - General performance benchmarks
examples/
  load_test.rs          - Load testing tool
  test_ipv6.rs          - IPv6 connectivity test
  thread_monitor.rs     - Thread monitoring utility
  thread_test.rs        - Thread pool testing
```

### Core Components

- **Server** (`src/server.rs`): Central orchestrator managing brokers, sessions, routing, and shutdown. Exposes `start(config)` and `stop()`.
- **Broker** (`src/broker.rs`): Protocol-specific network listeners (TCP, TLS). Accepts connections and spawns Session tasks.
- **Session** (`src/session.rs`): Individual client connection handler. Manages the full MQTT session lifecycle: CONNECT, packet handling, QoS state machines, keep-alive, will messages, and clean disconnect.
- **Router** (`src/router.rs`): Message routing and subscription management using bidirectional mapping (filter->sessions, session->filters). Supports `+` single-level and `#` multi-level wildcards.
- **Protocol** (`src/protocol/`): MQTT v3.1.1 packet parsing and serialization. All 14 packet types implemented.
- **Transport** (`src/transport.rs`): `AsyncStream` trait abstracting over `TcpStream` and `TlsStream`.
- **Config** (`src/config.rs`): TOML-based configuration with custom serde deserializers (e.g., listen accepts string or array).
- **Auth** (`src/auth/`): Pluggable authentication (`Authenticator` trait) and authorization (`Authorizer` trait) with `allowall` and file-based backends.
- **Storage** (`src/storage/`): Pluggable persistence (`Storage` trait via async_trait) with `memory` and `sqlite` backends.

### Key Design Patterns

1. **CancellationToken Architecture**: Uses `tokio_util::sync::CancellationToken` throughout for race-condition-free shutdown
2. **Half-connected Sessions**: Tracked separately until CONNECT received to manage incomplete connections
3. **Stream Passing**: Packet handlers receive stream reference to avoid write lock deadlocks
4. **Thread-safe Cleanup**: Lock-based swap pattern for safe concurrent operations
5. **Event-driven**: `tokio::select!` for responsive packet and shutdown handling
6. **Session Management**: SessionId starts as `__anon_$uuid`, becomes `__client_$clientId` after CONNECT
7. **Command Channel Pattern**: Sessions use `mpsc::Sender<Command>` for external control (takeover, disconnect)
8. **Build-time Git Revision**: `build.rs` embeds git hash into the binary via `GIT_REVISION` env var

### Current Status (Milestone 5 - Completed)

**Milestone 1** (completed): Full MQTTv3 Server (QoS=0, no persistency/auth)
- Event-driven architecture with tokio::select!
- Race-condition-free shutdown using CancellationToken
- Complete MQTT v3.1.1 packet handling (all 14 packet types)
- Message routing with topic filtering and wildcard support (`+`, `#`)
- Clean session logic with session takeover
- Keep-alive mechanism
- Retained messages with wildcard delivery
- Will messages (Last Will and Testament)

**Milestone 2** (completed): QoS=1 Support
- PUBLISH/PUBACK message flow with retransmission and DUP flag
- Multiple in-flight messages, duplicate detection, configurable retry

**Milestone 3** (completed): Persistence Layer
- Unified `Storage` trait with `InMemoryStorage` and `SqliteStorage` backends
- Session, subscription, in-flight message, and retained message persistence
- Atomic session state save (all-or-nothing)

**Milestone 4** (completed): Security
- TLS/SSL via `tls://` listener prefix (tokio-rustls)
- Username/password authentication (file-based)
- Topic-based ACLs for publish/subscribe access control
- Pluggable auth/ACL backends (`allowall`, `passwordfile`, `aclfile`)

**Milestone 5** (completed): QoS=2 Support
- PUBREC/PUBREL/PUBCOMP four-step handshake
- QoS=2 state machine (`AwaitingPubRec`, `AwaitingPubComp`)
- Inbound and outbound QoS=2 handling with retransmission
- QoS=2 state persistence across restarts

### Development Roadmap

| Milestone | Status | Description |
|-----------|--------|-------------|
| 1 | Done | Full MQTTv3 Server (QoS=0) |
| 2 | Done | QoS=1 Support |
| 3 | Done | Persistence Layer |
| 4 | Done | Security (TLS, Auth, ACLs) |
| 5 | Done | QoS=2 Support |
| 6 | Next | Observability (Prometheus, Grafana) |
| 7 | Planned | Flow Control & Production Features |
| v1.0 | Planned | Production Ready |
| v2.0 | Future | MQTT 5.0 Support |
| v3.0 | Future | Clustering & High Availability |

### Configuration

TOML-based configuration system. Key sections:

```toml
listen = ["tcp://0.0.0.0:1883", "tls://0.0.0.0:8883"]  # or single string

retained_message_limit = 10000       # Max retained messages
max_retransmission_limit = 10        # QoS retry limit
retransmission_interval_ms = 5000    # QoS retry interval (min 500ms)

[persistence]
backend = "memory"                   # "memory" or "sqlite"
database_path = "iotd.db"            # SQLite path
session_expiry_seconds = 86400       # 24h default

[auth]
backend = "allowall"                 # "allowall" or "passwordfile"
password_file = "passwd"

[acl]
backend = "allowall"                 # "allowall" or "aclfile"
acl_file = "acl.conf"

[tls]
cert_file = "server.crt"            # Required for tls:// listeners
key_file = "server.key"
```

Default (no config file): listens on `127.0.0.1:1883`, memory storage, allow-all auth.

### CI/CD

GitHub Actions CI (`.github/workflows/ci.yml`) runs on push/PR to main:
- `cargo test --verbose`
- `cargo clippy -- -D warnings`
- `cargo fmt -- --check`

Additional workflows: `build-release.yml`, `release.yml`.

### Testing Strategy

- **136 tests** across 6 test files plus inline unit tests
- Integration tests spawn test servers on ports 18831, 18832, 18833 to avoid conflicts
- Tests construct raw MQTT packets and verify byte-level protocol compliance
- TLS tests generate self-signed certs at runtime using the `rcgen` crate
- QoS tests verify full delivery flows including retransmission and state recovery

### Key Technical Notes

- **Deadlock Prevention**: Session packet handlers accept stream parameter to avoid double-locking
- **Session Cleanup**: `cleanup_and_exit()` removes half-connected sessions only if not connected
- **Shutdown Sequence**: Server -> Brokers -> Sessions with proper ordering
- **Error Handling**: `handle_message` failures terminate session loop
- **Import Pattern**: Use `use crate::protocol::packet;` for module-level imports
- **Router Architecture**: Uses `RwLock` with bidirectional mapping (filter->sessions, session->filters) for efficient routing and cleanup
- **Wildcard Matching**: MQTT-compliant topic matching with `+` (single-level) and `#` (multi-level)
- **Session Takeover**: `clean_session=false` clients can take over existing sessions with DISCONNECT notification
- **Keep-Alive Monitoring**: Automatic disconnection of inactive clients based on keep-alive timeout (1.5x multiplier per spec)
- **QoS State Machines**: `InflightMessage` tracks QoS=1 (PUBACK) and QoS=2 (PUBREC->PUBREL->PUBCOMP) states
- **Packet ID Generation**: Sequential `next_packet_id` in session, wraps around at u16::MAX
- **Spec Compliance**: Strict adherence to MQTT 3.1.1 specification without over-engineering

### Dependencies

Key runtime dependencies:
- `tokio` (full) - Async runtime
- `tokio-util` - CancellationToken
- `tokio-rustls` / `rustls-pemfile` - TLS support
- `bytes` - Efficient byte buffer handling
- `tracing` / `tracing-subscriber` - Structured logging
- `serde` / `toml` - Configuration
- `rusqlite` (bundled) - SQLite persistence
- `dashmap` - Concurrent HashMap
- `uuid` - Session ID generation
- `async-trait` - Async trait support
- `anyhow` / `thiserror` - Error handling
- `chrono` - Timestamps for persistence

### Common Issues and Solutions

1. **Deadlock in packet handling**: Pass stream reference to handlers instead of acquiring lock again
2. **Race conditions in shutdown**: Use CancellationToken instead of Notify for state-maintaining cancellation
3. **Session cleanup ordering**: Half-connected removal before server registration
4. **Stream write failures**: Terminate session loop on message delivery errors
5. **SQLite bundled compilation**: Uses `rusqlite` with `bundled` feature; first build compiles sqlite3 from source (slow)
6. **Test port conflicts**: Each test file uses distinct ports (18831+) to avoid bind failures in parallel test runs
