# Gemini Code Assistant Context

This document provides context for the Gemini Code Assistant to understand the IoTHub project.

## Project Overview

IoTHub is a high-performance, extensible MQTT server built in Rust with Tokio. It's designed for scalability and reliability, featuring a modern, event-driven asynchronous architecture. The server supports multiple transport protocols and is engineered to handle a large number of concurrent connections with a strong focus on race-condition-free shutdown and robust session management.

The project development follows a milestone-based approach, incrementally adding features to ensure a stable foundation.

## Core Architecture

The architecture is based on a `Server` → `Broker` → `Session` hierarchy, designed for clear separation of concerns and scalability.

*   **Server**: The central orchestrator. It manages the lifecycle of brokers, handles session registration, and coordinates a graceful, race-condition-free shutdown process using `CancellationToken`.
*   **Broker**: A network listener for a specific protocol and address (e.g., TCP). It accepts incoming connections, creates `Session` tasks, and tracks them as "half-connected" until the initial MQTT CONNECT packet is received.
*   **Session**: Represents a single connected MQTT client. It manages the client's entire lifecycle, from the initial anonymous state (`__anon_$uuid`) to a client-identified state (`__client_$clientId`). It uses an event-driven `tokio::select!` loop to handle incoming packets, outgoing messages, and shutdown signals. A key design feature is passing the network stream as a mutable reference to packet handlers to prevent deadlocks.
*   **Router**: Manages topic subscriptions and routes PUBLISH messages to the appropriate subscribed sessions. It uses the internal `sessionId` for routing to avoid conflicts with client-provided IDs.
*   **Transport Abstraction**: A trait-based system (`AsyncListener`, `AsyncStream`) allows the server to transparently support different network protocols like TCP, with plans for TLS and WebSocket.

## Key Design Decisions

*   **`CancellationToken` for Shutdown**: Replaced `tokio::sync::Notify` to eliminate race conditions and ensure a reliable, stateful shutdown signal is propagated through all components (Server, Broker, Session).
*   **Half-Connected Session Tracking**: The `Broker` tracks newly accepted connections in a `half_connected_sessions` map. This ensures that even sessions that fail to send a CONNECT packet are properly cleaned up.
*   **Stream Deadlock Prevention**: Packet handling functions within a `Session` now accept a mutable reference to the network stream (`&mut dyn AsyncStream`). This avoids scenarios where a function could try to acquire a lock on the stream that it already holds, preventing deadlocks.
*   **Thread-Safe Cleanup**: To avoid deadlocks during shutdown (e.g., a session trying to unregister itself while the server is iterating over the session map), the `unregister_session` logic was simplified to only handle removal from the server's state. The server now manages the shutdown of all components in a clear, sequential order.

## Project Structure

```
iothub/
├── src/
│   ├── protocol/          # MQTT protocol implementation (packets, codecs)
│   ├── server.rs          # Core server orchestration and lifecycle management
│   ├── broker.rs          # Manages network listeners and half-connected sessions
│   ├── session.rs         # Client session handling, state, and packet processing
│   ├── router.rs          # Message routing and subscription management
│   ├── transport.rs       # Transport abstraction (TCP, TLS, etc.)
│   ├── config.rs          # Configuration loading and management
│   ├── storage/           # (Planned) Persistence layer
│   └── auth/              # (Planned) Authentication/Authorization logic
├── docs/                  # Architecture, roadmap, and design documents
├── tests/                 # Integration and end-to-end tests
└── ...
```

## Development Roadmap & Status

The project is currently in **Milestone 1**, focused on building a complete MQTT v3.1.1 server with QoS 0.

*   **Completed Features**:
    *   Event-driven architecture with `tokio::select!`.
    *   `CancellationToken`-based race-free shutdown.
    *   Half-connected session tracking and cleanup.
    *   Stream deadlock prevention.
    *   UNIX signal handling (SIGINT for graceful shutdown).
    *   Basic MQTT packet handling (CONNECT, PUBLISH, SUBSCRIBE, etc.).

*   **In Progress / Next Steps for Milestone 1**:
    *   Implementing the core message routing system in `router.rs`.
    *   Full implementation of `clean_session` logic.
    *   Handling of retained messages and Last Will and Testament (LWT).
    *   Implementing a keep-alive timeout mechanism.

*   **Future Milestones**:
    *   **M2**: QoS 1 Support (in-memory).
    *   **M3**: QoS 2 Support & Persistent Storage.
    *   **M4**: Basic Authentication.
    *   **M5**: Enhanced Transport Layer (TLS, WebSocket).
    *   **M6**: Pluggable Architecture for auth and storage.
    *   **M7**: Production-readiness (metrics, logging, docs).

## Getting Started

### Prerequisites
- Rust 1.75+

### Build
```bash
cargo build --release
```

### Run
```bash
cargo run
```

### Test
```bash
cargo test
```