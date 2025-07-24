# IoTD Performance Benchmarks

This document describes the performance benchmarking tools available for IoTD.

## Quick Start

Run all benchmarks:
```bash
./run_benchmarks.sh
```

## Available Benchmarks

### 1. Load Test Tool (`examples/load_test.rs`)

A comprehensive load testing tool that measures:
- Memory footprint per connection
- Message throughput (messages/second)
- Network throughput (MB/second)
- Connection establishment rate

#### Usage

```bash
cargo run --release --example load_test -- [server_addr] [publishers] [subscribers] [payload_size] [rate] [duration]
```

Parameters:
- `server_addr` - MQTT server address (default: 127.0.0.1:1883)
- `publishers` - Number of publisher clients (default: 10)
- `subscribers` - Number of subscriber clients (default: 10)
- `payload_size` - Message payload size in bytes (default: 100)
- `rate` - Messages per second per publisher (default: 100)
- `duration` - Test duration in seconds (default: 30)

#### Example Tests

**Memory Footprint with 1000 connections:**
```bash
cargo run --release --example load_test -- localhost:1883 50 950 64 1 30
```
This creates 1000 total connections (50 publishers + 950 subscribers) with minimal message traffic to measure memory usage.

**Maximum Throughput:**
```bash
cargo run --release --example load_test -- localhost:1883 10 10 64 1000 30
```
This tests maximum message throughput with 10 publishers sending 1000 msg/sec each.

**Realistic IoT Scenario:**
```bash
cargo run --release --example load_test -- localhost:1883 100 20 256 10 60
```
This simulates 100 IoT devices publishing telemetry data at 10 Hz to 20 monitoring applications.

### 2. Criterion Benchmarks (`benches/performance_benchmark.rs`)

Detailed microbenchmarks using the Criterion framework:

```bash
cargo bench
```

This runs benchmarks for:
- Memory footprint with various connection counts
- Throughput under different scenarios
- Connection establishment rate

Results are saved in `target/criterion/` with HTML reports.

## Expected Results

Based on actual benchmarks with ERROR log level (to reduce logging overhead):

### Memory Footprint
- **Base server**: ~2-3 MB
- **Per connection (idle)**: ~8-10 KB
- **Per connection (active)**: ~16-65 KB
- **1000 connections**: ~10-20 MB total RSS

### Throughput (single instance)
- **Small messages (64B)**: 4,000-9,000 msg/sec
- **Medium messages (100B)**: 2,000-4,000 msg/sec
- **Large messages (1KB)**: 1,000-2,000 msg/sec

### Connection Rate
- **Establishment**: 100-500 connections/sec
- **Concurrent**: 1,000+ simultaneous connections tested

### Actual Test Results
Test environment: **Mac Studio 2025 with M4 Max (16 cores)**

With `RUST_LOG=error` to minimize logging overhead:

1. **1000 Connections Test**:
   - Memory: 8.45 KB per connection
   - Total growth: 8.25 MB for 1000 connections
   - Message routing: 8,600+ msg/sec to 950 subscribers

2. **High Throughput Test** (10 connections):
   - Send rate: 4,353 msg/sec
   - Receive rate: 8,823 msg/sec  
   - Network throughput: 1.97 MB/sec

3. **Moderate Load** (20 connections):
   - Send rate: 927 msg/sec (near 1000 msg/sec target)
   - Memory: 64.8 KB per active connection

Note: IoTD uses Tokio's default multi-threaded runtime (16 threads on M4 Max), but performance may be limited by:
- Lock contention on the central Router (single RwLock)
- Logging overhead (use RUST_LOG=error for benchmarks)
- Client test tool limitations (single process)

## Performance Factors

Performance depends on:
1. **Hardware**: CPU cores, RAM, network interface
   - Test results above are from Mac Studio 2025 (M4 Max 16-core)
   - Performance will vary on different hardware
   - IoTD uses multi-threaded Tokio runtime but may not fully utilize all cores due to lock contention
2. **Message size**: Smaller messages = higher msg/sec rate
3. **Topic complexity**: Wildcard subscriptions add overhead
4. **QoS level**: Currently QoS 0 only (fastest)
5. **Network latency**: Local vs remote clients
6. **Logging level**: INFO logging significantly impacts performance
   - Use `RUST_LOG=warn` or `RUST_LOG=error` for benchmarks
   - INFO level can reduce throughput by 50-90%

## Monitoring During Tests

Monitor system resources:
```bash
# CPU and memory
htop

# Network traffic
iftop -i lo  # for localhost testing

# File descriptors
lsof -p $(pgrep iotd) | wc -l
```

## Tuning for Performance

### System Limits
```bash
# Increase file descriptor limit
ulimit -n 65536

# Check current limits
ulimit -a
```

### Server Configuration
Edit `Cargo.toml` for release optimizations:
```toml
[profile.release]
lto = true
codegen-units = 1
panic = "abort"
```

### Network Tuning (Linux)
```bash
# Increase socket buffer sizes
sudo sysctl -w net.core.rmem_max=134217728
sudo sysctl -w net.core.wmem_max=134217728
sudo sysctl -w net.ipv4.tcp_rmem="4096 87380 134217728"
sudo sysctl -w net.ipv4.tcp_wmem="4096 65536 134217728"
```

## Interpreting Results

The load test tool provides:
- **Real-time stats**: Message rates every 5 seconds
- **Memory growth**: Initial vs final memory usage
- **Connection metrics**: Success/failure rates
- **Network throughput**: Total bytes transferred

Look for:
- Stable message rates (no degradation over time)
- Linear memory growth with connections
- Low connection failure rates
- Consistent latency