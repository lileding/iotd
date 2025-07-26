use std::thread;
use std::time::Duration;

fn main() {
    println!("Checking Tokio runtime configuration...");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    runtime.block_on(async {
        println!(
            "Available parallelism: {}",
            std::thread::available_parallelism().unwrap()
        );

        // Get current thread ID
        let main_thread = thread::current().id();
        println!("Main task thread: {:?}", main_thread);

        // Spawn multiple tasks and check their thread IDs
        let mut handles = vec![];

        for i in 0..20 {
            let handle = tokio::spawn(async move {
                let thread_id = thread::current().id();
                // Do some work to ensure the task isn't too short
                tokio::time::sleep(Duration::from_millis(10)).await;
                (i, thread_id)
            });
            handles.push(handle);
        }

        // Collect results
        let mut thread_ids = std::collections::HashSet::new();
        for handle in handles {
            let (task_num, thread_id) = handle.await.unwrap();
            thread_ids.insert(thread_id);
            println!("Task {} ran on thread {:?}", task_num, thread_id);
        }

        println!("\nSummary:");
        println!("Total unique threads used: {}", thread_ids.len());
        println!(
            "Expected threads (CPU cores): {}",
            std::thread::available_parallelism().unwrap()
        );
    });
}
