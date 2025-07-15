# IoTHub - High-Performance MQTT Server

A high-performance MQTT server implementation in Rust using Tokio, designed for low latency, minimal memory footprint, and high throughput.

## Features

### Stone 1 (Current) ✅
- Basic pub/sub messaging with QoS level 0
- No authentication required  
- MQTT v3.1.1 protocol support
- Topic wildcards (+ and #)
- Retained messages
- Clean session support

### Roadmap
- **Stone 2**: Drain and willing message support
- **Stone 3**: QoS level 1 and enhanced retained messages
- **Stone 4**: QoS level 2 support
- **Stone 5**: Authentication (no authorization)
- **Stone 6**: Persistence interface with plug-in storage
- **Stone 7**: Authorization support
- **Future**: MQTT v5.0 support

## Architecture

```
iothub/
├── src/
│   ├── protocol/          # MQTT protocol implementation
│   ├── server/            # Core server logic
│   ├── storage/           # Persistence layer (Stone 6)
│   ├── auth/              # Authentication/Authorization (Stone 5-7)
│   └── config/            # Configuration management
├── tests/                 # Integration tests
├── benches/              # Performance benchmarks
└── docker/               # Docker configuration
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

Currently uses hardcoded configuration. Future versions will support:
- Configuration files (TOML/JSON)
- Environment variables
- Command-line arguments

## License

MIT