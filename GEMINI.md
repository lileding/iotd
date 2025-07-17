# Gemini Code Assistant Context

This document provides context for the Gemini Code Assistant to understand the IoTHub project.

## Project Overview

IoTHub is a high-performance, extensible MQTT server built in Rust with Tokio. It's designed for scalability and reliability, featuring a modern asynchronous architecture that supports multiple transport protocols and a large number of concurrent connections.

The project development follows a "stone" approach, incrementally adding features to ensure a stable foundation.

## Core Architecture

The architecture follows a `Server` → `Broker` → `Session` hierarchy.

*   **Server**: The central orchestrator that manages brokers, sessions, and global state (configuration, routing, shutdown).
*   **Broker**: A network listener for a specific protocol and address (e.g., TCP, WebSocket). It accepts incoming connections and hands them off to new `Session` tasks.
*   **Session**: Represents a single connected MQTT client. It handles the client's entire lifecycle, including packet processing, state management, and communication with the `Router`.
*   **Router**: Manages topic subscriptions and routes PUBLISH messages to the appropriate subscribed sessions. It uses a unique `sessionId` for internal routing to handle client ID conflicts.
*   **Transport Abstraction**: A trait-based system (`AsyncListener`, `AsyncStream`) allows the server to support different network protocols (TCP, TLS, WebSocket) transparently.

## Project Structure

```
iothub/
├── src/
│   ├── protocol/          # MQTT protocol implementation (packets, codecs)
│   ├── server.rs          # Core server orchestration and lifecycle management
│   ├── session.rs         # Client session handling and state
│   ├── router.rs          # Message routing and subscription management
│   ├── transport.rs       # Transport abstraction (TCP, TLS, etc.)
│   ├── config.rs          # Configuration loading and management
│   ├── storage/           # Persistence layer (pluggable backends)
│   └── auth/              # Authentication/Authorization logic
├── docs/                  # Architecture, roadmap, and design documents
├── tests/                 # Integration and end-to-end tests
├── benches/               # Performance benchmarks
└── docker/                # Docker configuration
```

## Key Features & Roadmap

The project is developed in stages ("stones"):

*   **Stone 1 (Complete)**: Basic MQTT v3.1.1 support, QoS 0, pub/sub, retained messages, and clean sessions.
*   **Stone 2 (Complete)**: Multi-broker architecture, graceful shutdown with connection draining, unique `sessionId` for robust session management, and a transport abstraction layer.
*   **Stone 3 (Planned)**: QoS 1, Last Will and Testament (LWT).
*   **Stone 4 (Planned)**: QoS 2 support.
*   **Stone 5 (Planned)**: Pluggable authentication (username/password, TLS).
*   **Stone 6 (Planned)**: Pluggable persistence layer for session state and messages.
*   **Future Stones**: Authorization, MQTT v5.0, multi-protocol support (WebSocket), and clustering.

## Getting Started

### Prerequisites
- Rust 1.75+
- Tokio runtime

### Build
```bash
cargo build --release
```

### Run
```bash
cargo run
```
The server starts on `localhost:1883` by default.

### Test
```bash
cargo test
```

### Docker
```bash
# Build
docker build -f docker/Dockerfile -t iothub .

# Run
docker run -p 1883:1883 iothub
```

## Configuration

The server is configured via a `config.toml` file, environment variables, or command-line arguments. Key settings include listen addresses, connection limits, and timeouts.
