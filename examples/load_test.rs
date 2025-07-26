use futures::future::join_all;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::time::{sleep, timeout, Duration, Instant};

// Statistics tracking
#[derive(Default)]
struct Stats {
    connections_established: AtomicU64,
    connections_failed: AtomicU64,
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
}

// Memory measurement utilities
#[cfg(target_os = "linux")]
fn get_memory_usage() -> (u64, u64) {
    use std::fs;
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut vm_rss = 0;
    let mut vm_size = 0;

    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                vm_rss = parts[1].parse::<u64>().unwrap_or(0) * 1024;
            }
        } else if line.starts_with("VmSize:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                vm_size = parts[1].parse::<u64>().unwrap_or(0) * 1024;
            }
        }
    }
    (vm_rss, vm_size)
}

#[cfg(target_os = "macos")]
fn get_memory_usage() -> (u64, u64) {
    use std::process::Command;
    let output = Command::new("ps")
        .args(&["-o", "rss=,vsz=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();

    let output_str = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = output_str.trim().split_whitespace().collect();

    let rss = parts
        .get(0)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;
    let vsz = parts
        .get(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;

    (rss, vsz)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn get_memory_usage() -> (u64, u64) {
    (0, 0)
}

// MQTT packet helpers
fn create_connect_packet(client_id: &str) -> Vec<u8> {
    let mut packet = vec![
        0x10, // CONNECT packet type
        0x00, // Will be filled with remaining length
        0x00,
        0x04, // Protocol name length
        b'M',
        b'Q',
        b'T',
        b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00,
        0x3C, // Keep alive (60 seconds)
        (client_id.len() >> 8) as u8,
        client_id.len() as u8, // Client ID length
    ];
    packet.extend_from_slice(client_id.as_bytes());

    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

fn create_publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x30]; // PUBLISH packet type, QoS 0
    let mut remaining_data = Vec::new();

    // Topic name
    remaining_data.extend_from_slice(&[(topic.len() >> 8) as u8, topic.len() as u8]);
    remaining_data.extend_from_slice(topic.as_bytes());

    // Payload
    remaining_data.extend_from_slice(payload);

    // Remaining length (simplified for payloads < 128 bytes)
    if remaining_data.len() < 128 {
        packet.push(remaining_data.len() as u8);
    } else {
        // Encode larger remaining length
        let mut len = remaining_data.len();
        while len > 0 {
            let mut byte = (len & 0x7F) as u8;
            len >>= 7;
            if len > 0 {
                byte |= 0x80;
            }
            packet.push(byte);
        }
    }

    packet.extend_from_slice(&remaining_data);
    packet
}

fn create_subscribe_packet(packet_id: u16, topic: &str) -> Vec<u8> {
    let mut packet = vec![
        0x82, // SUBSCRIBE packet type with flags
        0x00, // Will be filled with remaining length
        (packet_id >> 8) as u8,
        packet_id as u8, // Packet ID
        (topic.len() >> 8) as u8,
        topic.len() as u8, // Topic length
    ];
    packet.extend_from_slice(topic.as_bytes());
    packet.push(0x00); // QoS 0

    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

fn create_pingreq_packet() -> Vec<u8> {
    vec![0xC0, 0x00] // PINGREQ with 0 remaining length
}

async fn create_subscriber(
    server_addr: &str,
    client_id: String,
    topic: &str,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = TcpStream::connect(server_addr).await?;

    // Send CONNECT
    let connect = create_connect_packet(&client_id);
    stream.write_all(&connect).await?;
    stats
        .bytes_sent
        .fetch_add(connect.len() as u64, Ordering::Relaxed);

    // Read CONNACK
    let mut connack = [0u8; 4];
    stream.read_exact(&mut connack).await?;
    stats.bytes_received.fetch_add(4, Ordering::Relaxed);

    // Send SUBSCRIBE
    let subscribe = create_subscribe_packet(1, topic);
    stream.write_all(&subscribe).await?;
    stats
        .bytes_sent
        .fetch_add(subscribe.len() as u64, Ordering::Relaxed);

    // Read SUBACK
    let mut suback = [0u8; 5];
    stream.read_exact(&mut suback).await?;
    stats.bytes_received.fetch_add(5, Ordering::Relaxed);

    stats
        .connections_established
        .fetch_add(1, Ordering::Relaxed);

    // Keep reading messages
    let mut buffer = vec![0u8; 4096];
    while running.load(Ordering::Relaxed) {
        match timeout(Duration::from_secs(5), stream.read(&mut buffer)).await {
            Ok(Ok(n)) if n > 0 => {
                stats.messages_received.fetch_add(1, Ordering::Relaxed);
                stats.bytes_received.fetch_add(n as u64, Ordering::Relaxed);
            }
            _ => {
                // Send PINGREQ to keep connection alive
                let pingreq = create_pingreq_packet();
                if stream.write_all(&pingreq).await.is_err() {
                    break;
                }
                stats
                    .bytes_sent
                    .fetch_add(pingreq.len() as u64, Ordering::Relaxed);
            }
        }
    }

    Ok(())
}

async fn create_publisher(
    server_addr: &str,
    client_id: String,
    topic: &str,
    payload_size: usize,
    rate: u64, // messages per second
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = TcpStream::connect(server_addr).await?;

    // Send CONNECT
    let connect = create_connect_packet(&client_id);
    stream.write_all(&connect).await?;
    stats
        .bytes_sent
        .fetch_add(connect.len() as u64, Ordering::Relaxed);

    // Read CONNACK
    let mut connack = [0u8; 4];
    stream.read_exact(&mut connack).await?;
    stats.bytes_received.fetch_add(4, Ordering::Relaxed);

    stats
        .connections_established
        .fetch_add(1, Ordering::Relaxed);

    // Prepare payload
    let payload = vec![b'X'; payload_size];
    let publish = create_publish_packet(topic, &payload);

    // Calculate interval
    let interval = Duration::from_micros(1_000_000 / rate);

    // Publish messages at specified rate
    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        if stream.write_all(&publish).await.is_ok() {
            stats.messages_sent.fetch_add(1, Ordering::Relaxed);
            stats
                .bytes_sent
                .fetch_add(publish.len() as u64, Ordering::Relaxed);
        } else {
            break;
        }

        let elapsed = start.elapsed();
        if elapsed < interval {
            sleep(interval - elapsed).await;
        }
    }

    Ok(())
}

async fn run_load_test(
    server_addr: &str,
    num_publishers: usize,
    num_subscribers: usize,
    payload_size: usize,
    messages_per_publisher_per_sec: u64,
    duration_secs: u64,
) {
    println!("=== IoTD Load Test ===");
    println!("Server: {}", server_addr);
    println!("Publishers: {}", num_publishers);
    println!("Subscribers: {}", num_subscribers);
    println!("Payload size: {} bytes", payload_size);
    println!(
        "Rate: {} msg/sec per publisher",
        messages_per_publisher_per_sec
    );
    println!("Duration: {} seconds", duration_secs);
    println!(
        "Total target rate: {} msg/sec",
        num_publishers as u64 * messages_per_publisher_per_sec
    );
    println!();

    let stats = Arc::new(Stats::default());
    let running = Arc::new(AtomicBool::new(true));

    // Measure initial memory
    let (initial_rss, initial_vsz) = get_memory_usage();
    println!(
        "Initial memory - RSS: {:.2} MB, VSZ: {:.2} MB",
        initial_rss as f64 / 1_048_576.0,
        initial_vsz as f64 / 1_048_576.0
    );

    // Create subscribers
    println!("Creating {} subscribers...", num_subscribers);
    let mut subscriber_tasks = Vec::new();
    for i in 0..num_subscribers {
        let server_addr = server_addr.to_string();
        let client_id = format!("sub{}", i);
        let stats = stats.clone();
        let running = running.clone();

        let task = tokio::spawn(async move {
            if let Err(e) = create_subscriber(
                &server_addr,
                client_id,
                "load/test/+",
                stats.clone(),
                running,
            )
            .await
            {
                stats.connections_failed.fetch_add(1, Ordering::Relaxed);
                eprintln!("Subscriber error: {}", e);
            }
        });
        subscriber_tasks.push(task);
    }

    // Wait for subscribers to connect
    sleep(Duration::from_secs(2)).await;

    // Create publishers
    println!("Creating {} publishers...", num_publishers);
    let mut publisher_tasks = Vec::new();
    for i in 0..num_publishers {
        let server_addr = server_addr.to_string();
        let client_id = format!("pub{}", i);
        let topic = format!("load/test/{}", i % 10); // Distribute across 10 topics
        let stats = stats.clone();
        let running = running.clone();

        let task = tokio::spawn(async move {
            if let Err(e) = create_publisher(
                &server_addr,
                client_id,
                &topic,
                payload_size,
                messages_per_publisher_per_sec,
                stats.clone(),
                running,
            )
            .await
            {
                stats.connections_failed.fetch_add(1, Ordering::Relaxed);
                eprintln!("Publisher error: {}", e);
            }
        });
        publisher_tasks.push(task);
    }

    // Wait for publishers to connect
    sleep(Duration::from_secs(2)).await;

    let total_connections = stats.connections_established.load(Ordering::Relaxed);
    println!("\nConnections established: {}", total_connections);

    // Measure memory after connections
    let (connected_rss, connected_vsz) = get_memory_usage();
    println!(
        "After connections - RSS: {:.2} MB, VSZ: {:.2} MB",
        connected_rss as f64 / 1_048_576.0,
        connected_vsz as f64 / 1_048_576.0
    );

    if total_connections > 0 {
        let rss_per_conn = (connected_rss - initial_rss) / total_connections;
        println!(
            "Memory per connection: {:.2} KB",
            rss_per_conn as f64 / 1024.0
        );
    }

    println!("\nRunning test for {} seconds...", duration_secs);

    // Run the test
    let start_time = Instant::now();
    let mut last_stats_time = start_time;
    let mut last_messages_sent = 0u64;
    let mut last_messages_received = 0u64;

    while start_time.elapsed() < Duration::from_secs(duration_secs) {
        sleep(Duration::from_secs(5)).await;

        // Print periodic stats
        let now = Instant::now();
        let elapsed = now.duration_since(last_stats_time).as_secs_f64();

        let messages_sent = stats.messages_sent.load(Ordering::Relaxed);
        let messages_received = stats.messages_received.load(Ordering::Relaxed);

        let send_rate = (messages_sent - last_messages_sent) as f64 / elapsed;
        let recv_rate = (messages_received - last_messages_received) as f64 / elapsed;

        println!(
            "[{:3.0}s] Send: {:.0} msg/s, Recv: {:.0} msg/s, Total sent: {}, Total recv: {}",
            start_time.elapsed().as_secs(),
            send_rate,
            recv_rate,
            messages_sent,
            messages_received
        );

        last_stats_time = now;
        last_messages_sent = messages_sent;
        last_messages_received = messages_received;
    }

    // Stop the test
    running.store(false, Ordering::Relaxed);
    println!("\nStopping test...");

    // Wait for tasks to complete
    let _ = join_all(subscriber_tasks).await;
    let _ = join_all(publisher_tasks).await;

    // Final statistics
    let total_duration = start_time.elapsed();
    let total_messages_sent = stats.messages_sent.load(Ordering::Relaxed);
    let total_messages_received = stats.messages_received.load(Ordering::Relaxed);
    let total_bytes_sent = stats.bytes_sent.load(Ordering::Relaxed);
    let total_bytes_received = stats.bytes_received.load(Ordering::Relaxed);

    // Final memory measurement
    let (final_rss, final_vsz) = get_memory_usage();

    println!("\n=== Final Statistics ===");
    println!("Test duration: {:.1} seconds", total_duration.as_secs_f64());
    println!(
        "Connections established: {}",
        stats.connections_established.load(Ordering::Relaxed)
    );
    println!(
        "Connections failed: {}",
        stats.connections_failed.load(Ordering::Relaxed)
    );
    println!();
    println!("Messages sent: {}", total_messages_sent);
    println!("Messages received: {}", total_messages_received);
    println!(
        "Average send rate: {:.0} msg/sec",
        total_messages_sent as f64 / total_duration.as_secs_f64()
    );
    println!(
        "Average receive rate: {:.0} msg/sec",
        total_messages_received as f64 / total_duration.as_secs_f64()
    );
    println!();
    println!(
        "Bytes sent: {:.2} MB",
        total_bytes_sent as f64 / 1_048_576.0
    );
    println!(
        "Bytes received: {:.2} MB",
        total_bytes_received as f64 / 1_048_576.0
    );
    println!(
        "Average throughput: {:.2} MB/sec",
        (total_bytes_sent + total_bytes_received) as f64
            / 1_048_576.0
            / total_duration.as_secs_f64()
    );
    println!();
    println!("Memory usage:");
    println!(
        "  Initial - RSS: {:.2} MB, VSZ: {:.2} MB",
        initial_rss as f64 / 1_048_576.0,
        initial_vsz as f64 / 1_048_576.0
    );
    println!(
        "  Final   - RSS: {:.2} MB, VSZ: {:.2} MB",
        final_rss as f64 / 1_048_576.0,
        final_vsz as f64 / 1_048_576.0
    );
    println!(
        "  Growth  - RSS: {:.2} MB, VSZ: {:.2} MB",
        (final_rss - initial_rss) as f64 / 1_048_576.0,
        (final_vsz - initial_vsz) as f64 / 1_048_576.0
    );

    if total_connections > 0 {
        println!(
            "  Per connection: {:.2} KB",
            (final_rss - initial_rss) as f64 / 1024.0 / total_connections as f64
        );
    }
}

fn main() {
    // Set logging to WARN or ERROR to reduce overhead during benchmarks
    std::env::set_var("RUST_LOG", "warn");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help") {
        println!(
            "Usage: {} [server_addr] [publishers] [subscribers] [payload_size] [rate] [duration]",
            args[0]
        );
        println!();
        println!("Arguments:");
        println!("  server_addr  - MQTT server address (default: 127.0.0.1:1883)");
        println!("  publishers   - Number of publisher clients (default: 10)");
        println!("  subscribers  - Number of subscriber clients (default: 10)");
        println!("  payload_size - Message payload size in bytes (default: 100)");
        println!("  rate         - Messages per second per publisher (default: 100)");
        println!("  duration     - Test duration in seconds (default: 30)");
        println!();
        println!("Examples:");
        println!("  {} 127.0.0.1:1883 10 10 100 100 30", args[0]);
        println!(
            "  {} localhost:1883 50 950 64 10 60  # 1000 connections test",
            args[0]
        );
        return;
    }

    let server_addr = args.get(1).map(|s| s.as_str()).unwrap_or("127.0.0.1:1883");
    let num_publishers = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let num_subscribers = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let payload_size = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(100);
    let rate = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(100);
    let duration = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(30);

    // Create runtime and run test
    let runtime = Runtime::new().unwrap();
    runtime.block_on(run_load_test(
        server_addr,
        num_publishers,
        num_subscribers,
        payload_size,
        rate,
        duration,
    ));
}
