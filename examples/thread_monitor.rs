use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::collections::HashMap;
use std::thread;
use tokio::time::{sleep, Duration};

static THREAD_COUNTER: AtomicUsize = AtomicUsize::new(0);

// Thread-local storage to track which threads are processing work
thread_local! {
    static THREAD_ID: usize = THREAD_COUNTER.fetch_add(1, Ordering::SeqCst);
}

fn get_thread_id() -> usize {
    THREAD_ID.with(|id| *id)
}

async fn monitor_iotd_threads() {
    println!("Monitoring IoTD thread usage...");
    println!("Available CPU cores: {}", std::thread::available_parallelism().unwrap());
    
    // Connect multiple clients
    let mut clients = Vec::new();
    
    for i in 0..50 {
        let client = tokio::spawn(async move {
            let thread_stats = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            
            match tokio::net::TcpStream::connect("127.0.0.1:1883").await {
                Ok(mut stream) => {
                    // Track which thread handles this connection
                    let thread_id = get_thread_id();
                    let mut stats = thread_stats.lock().await;
                    *stats.entry(thread_id).or_insert(0) += 1;
                    
                    // Send CONNECT
                    let connect = create_connect_packet(&format!("monitor{}", i));
                    use tokio::io::AsyncWriteExt;
                    let _ = stream.write_all(&connect).await;
                    
                    // Keep connection alive
                    sleep(Duration::from_secs(5)).await;
                    
                    println!("Client {} handled by thread {}", i, thread_id);
                }
                Err(e) => {
                    eprintln!("Connection error: {}", e);
                }
            }
            
            thread_stats
        });
        
        clients.push(client);
        
        // Small delay between connections
        sleep(Duration::from_millis(10)).await;
    }
    
    // Wait for all clients
    let mut all_thread_ids = std::collections::HashSet::new();
    for client in clients {
        if let Ok(stats) = client.await {
            let stats = stats.lock().await;
            for (thread_id, _) in stats.iter() {
                all_thread_ids.insert(*thread_id);
            }
        }
    }
    
    println!("\nThread usage summary:");
    println!("Unique threads used: {}", all_thread_ids.len());
    println!("Thread IDs: {:?}", all_thread_ids);
}

fn create_connect_packet(client_id: &str) -> Vec<u8> {
    let mut packet = vec![
        0x10, // CONNECT packet type
        0x00, // Will be filled with remaining length
        0x00, 0x04, // Protocol name length
        b'M', b'Q', b'T', b'T', // Protocol name "MQTT"
        0x04, // Protocol level (3.1.1)
        0x02, // Connect flags (clean session)
        0x00, 0x3C, // Keep alive (60 seconds)
        (client_id.len() >> 8) as u8, client_id.len() as u8, // Client ID length
    ];
    packet.extend_from_slice(client_id.as_bytes());
    
    // Calculate and set remaining length
    let remaining_length = (packet.len() - 2) as u8;
    packet[1] = remaining_length;
    packet
}

#[tokio::main]
async fn main() {
    // First ensure IoTD is running
    println!("Make sure IoTD is running on port 1883");
    println!("You can start it with: RUST_LOG=error ./target/release/iotd");
    println!();
    
    sleep(Duration::from_secs(2)).await;
    
    monitor_iotd_threads().await;
}