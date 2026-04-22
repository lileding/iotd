use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

// Memory measurement utilities
#[cfg(target_os = "linux")]
fn get_memory_usage() -> u64 {
    use std::fs;
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].parse::<u64>().unwrap_or(0) * 1024;
            }
        }
    }
    0
}

#[cfg(target_os = "macos")]
fn get_memory_usage() -> u64 {
    use std::process::Command;
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    let rss_kb = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    rss_kb * 1024
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn get_memory_usage() -> u64 {
    0
}

fn create_connect_packet(client_id: &str) -> Vec<u8> {
    let mut packet = vec![
        0x10,
        0x00,
        0x00,
        0x04,
        b'M',
        b'Q',
        b'T',
        b'T',
        0x04,
        0x02,
        0x00,
        0x3C,
        (client_id.len() >> 8) as u8,
        client_id.len() as u8,
    ];
    packet.extend_from_slice(client_id.as_bytes());
    packet[1] = (packet.len() - 2) as u8;
    packet
}

fn create_publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x30];
    let mut remaining_data = Vec::new();
    remaining_data.extend_from_slice(&[(topic.len() >> 8) as u8, topic.len() as u8]);
    remaining_data.extend_from_slice(topic.as_bytes());
    remaining_data.extend_from_slice(payload);
    packet.push(remaining_data.len() as u8);
    packet.extend_from_slice(&remaining_data);
    packet
}

fn create_subscribe_packet(packet_id: u16, topic: &str) -> Vec<u8> {
    let mut packet = vec![
        0x82,
        0x00,
        (packet_id >> 8) as u8,
        packet_id as u8,
        (topic.len() >> 8) as u8,
        topic.len() as u8,
    ];
    packet.extend_from_slice(topic.as_bytes());
    packet.push(0x00);
    packet[1] = (packet.len() - 2) as u8;
    packet
}

async fn wait_for_connack(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    if buf[0] != 0x20 || buf[3] != 0x00 {
        return Err("Invalid CONNACK".into());
    }
    Ok(())
}

/// Force-close connections with RST (SO_LINGER=0) to avoid TIME_WAIT port exhaustion.
fn force_close(connections: Vec<TcpStream>) {
    for stream in connections {
        let std_stream = stream.into_std().unwrap();
        let socket = socket2::Socket::from(std_stream);
        let _ = socket.set_linger(Some(Duration::from_secs(0)));
        drop(socket);
    }
}

async fn start_test_server() -> (iotd::server::ServerHandle, u16) {
    let mut config = iotd::config::Config::default();
    config.listen = vec!["127.0.0.1:0".to_string()];
    let server = iotd::server::start(config).await.unwrap();
    let address = server.address().await.unwrap();
    let port = address.split(':').last().unwrap().parse().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    (server, port)
}

fn benchmark_memory_footprint(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    let mut group = c.benchmark_group("memory_footprint");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for num_connections in [100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_connections),
            &num_connections,
            |b, &num_connections| {
                // Create server once, reuse across iterations
                let (server, port) = runtime.block_on(start_test_server());

                b.to_async(&runtime).iter(|| {
                    let server = &server;
                    async move {
                        let base_memory = get_memory_usage();

                        let mut connections = Vec::new();
                        for i in 0..num_connections {
                            let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
                                .await
                                .unwrap();
                            let connect = create_connect_packet(&format!("mem{}", i));
                            stream.write_all(&connect).await.unwrap();
                            wait_for_connack(&mut stream).await.unwrap();
                            connections.push(stream);
                        }

                        tokio::time::sleep(Duration::from_millis(500)).await;

                        let final_memory = get_memory_usage();
                        let memory_per_connection = if final_memory > base_memory {
                            (final_memory - base_memory) / num_connections as u64
                        } else {
                            0
                        };

                        println!(
                            "\n{} connections: base={:.2}MB, final={:.2}MB, per_conn={:.2}KB",
                            num_connections,
                            base_memory as f64 / 1_048_576.0,
                            final_memory as f64 / 1_048_576.0,
                            memory_per_connection as f64 / 1024.0
                        );

                        force_close(connections);
                        tokio::time::sleep(Duration::from_millis(200)).await;

                        let _ = server;
                        memory_per_connection
                    }
                });

                runtime.block_on(server.stop()).unwrap();
            },
        );
    }

    group.finish();
}

fn benchmark_throughput(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    let mut group = c.benchmark_group("throughput");
    group.sample_size(10);

    for (num_publishers, num_subscribers, payload_size) in [
        (1, 1, 64),
        (1, 10, 64),
        (10, 10, 64),
        (1, 1, 1024),
        (10, 10, 1024),
    ] {
        let scenario = format!(
            "{}pub_{}sub_{}bytes",
            num_publishers, num_subscribers, payload_size
        );

        group.bench_function(&scenario, |b| {
            let (server, port) = runtime.block_on(start_test_server());

            b.to_async(&runtime).iter(|| {
                let scenario = scenario.clone();
                async move {
                    // Create subscribers
                    let mut subscribers = Vec::new();
                    for i in 0..num_subscribers {
                        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
                            .await
                            .unwrap();
                        let connect = create_connect_packet(&format!("sub{}", i));
                        stream.write_all(&connect).await.unwrap();
                        wait_for_connack(&mut stream).await.unwrap();

                        let subscribe = create_subscribe_packet(1, "bench/topic");
                        stream.write_all(&subscribe).await.unwrap();
                        let mut suback = [0u8; 5];
                        stream.read_exact(&mut suback).await.unwrap();

                        subscribers.push(stream);
                    }

                    // Create publishers
                    let mut publishers = Vec::new();
                    for i in 0..num_publishers {
                        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
                            .await
                            .unwrap();
                        let connect = create_connect_packet(&format!("pub{}", i));
                        stream.write_all(&connect).await.unwrap();
                        wait_for_connack(&mut stream).await.unwrap();
                        publishers.push(stream);
                    }

                    let payload = vec![b'X'; payload_size];
                    let publish_packet = create_publish_packet("bench/topic", &payload);

                    let message_count = Arc::new(AtomicU64::new(0));
                    let receive_count = Arc::new(AtomicU64::new(0));
                    let duration = Duration::from_secs(5);
                    let start = Instant::now();

                    // Subscriber drain tasks — read and discard to prevent backpressure
                    let mut sub_tasks = Vec::new();
                    for mut subscriber in subscribers {
                        let recv = receive_count.clone();
                        sub_tasks.push(tokio::spawn(async move {
                            let mut buf = [0u8; 4096];
                            while start.elapsed() < duration {
                                match tokio::time::timeout(
                                    Duration::from_millis(100),
                                    subscriber.read(&mut buf),
                                )
                                .await
                                {
                                    Ok(Ok(0)) => break,
                                    Ok(Ok(n)) => {
                                        recv.fetch_add(n as u64, Ordering::Relaxed);
                                    }
                                    Ok(Err(_)) => break,
                                    Err(_) => {} // timeout, check elapsed again
                                }
                            }
                            subscriber
                        }));
                    }

                    // Publisher tasks
                    let mut pub_tasks = Vec::new();
                    for mut publisher in publishers {
                        let packet = publish_packet.clone();
                        let count = message_count.clone();
                        pub_tasks.push(tokio::spawn(async move {
                            while start.elapsed() < duration {
                                if publisher.write_all(&packet).await.is_ok() {
                                    count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            publisher
                        }));
                    }

                    // Collect all streams back for clean shutdown
                    let mut all_streams = Vec::new();
                    for task in pub_tasks {
                        all_streams.push(task.await.unwrap());
                    }
                    for task in sub_tasks {
                        all_streams.push(task.await.unwrap());
                    }

                    let total_messages = message_count.load(Ordering::Relaxed);
                    let elapsed = start.elapsed();
                    let messages_per_second = total_messages as f64 / elapsed.as_secs_f64();

                    println!(
                        "\n{}: {} messages in {:?} = {:.0} msg/sec",
                        scenario, total_messages, elapsed, messages_per_second
                    );

                    force_close(all_streams);
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    messages_per_second as u64
                }
            });

            runtime.block_on(server.stop()).unwrap();
        });
    }

    group.finish();
}

fn benchmark_connection_rate(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    let mut group = c.benchmark_group("connection_rate");
    group.sample_size(10);

    group.bench_function("connections_per_second", |b| {
        let (server, port) = runtime.block_on(start_test_server());

        b.to_async(&runtime).iter(|| async move {
            let duration = Duration::from_secs(5);
            let start = Instant::now();
            let mut connections = Vec::new();

            while start.elapsed() < duration {
                match TcpStream::connect(format!("127.0.0.1:{}", port)).await {
                    Ok(mut stream) => {
                        let connect =
                            create_connect_packet(&format!("client{}", connections.len()));
                        if stream.write_all(&connect).await.is_ok() {
                            connections.push(stream);
                        }
                    }
                    Err(_) => break,
                }
            }

            let elapsed = start.elapsed();
            let connection_count = connections.len() as u64;
            let connections_per_second = connection_count as f64 / elapsed.as_secs_f64();

            println!(
                "\nEstablished {} connections in {:?} = {:.0} conn/sec",
                connection_count, elapsed, connections_per_second
            );

            force_close(connections);
            connections_per_second as u64
        });

        runtime.block_on(server.stop()).unwrap();
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_memory_footprint,
    benchmark_throughput,
    benchmark_connection_rate
);
criterion_main!(benches);
