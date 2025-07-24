#!/bin/bash

echo "IoTD Performance Benchmarks"
echo "=========================="
echo

# Make sure we're in release mode
cargo build --release --example load_test

# Start the server in background with ERROR level logging for performance
echo "Starting IoTD server (with ERROR log level for performance)..."
RUST_LOG=error ./target/release/iotd &
SERVER_PID=$!
sleep 2

echo
echo "Test 1: Memory footprint with 1000 connections"
echo "----------------------------------------------"
./target/release/examples/load_test localhost:1883 50 950 64 1 30

echo
echo "Test 2: Maximum throughput (small messages)"
echo "------------------------------------------"
./target/release/examples/load_test localhost:1883 10 10 64 1000 30

echo
echo "Test 3: Maximum throughput (medium messages)"
echo "-------------------------------------------"
./target/release/examples/load_test localhost:1883 10 10 1024 100 30

echo
echo "Test 4: Stress test (many publishers)"
echo "-------------------------------------"
./target/release/examples/load_test localhost:1883 100 100 256 50 30

# Stop the server
echo
echo "Stopping server..."
kill $SERVER_PID
wait $SERVER_PID 2>/dev/null

echo
echo "Benchmark complete!"