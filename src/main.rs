mod rcu;

use rcu::Rcu;
use std::{thread, time::Duration};

fn main() {
    println!("=== RCU Demo with Automatic Cleanup ===");
    
    // Example 1: Basic usage with automatic cleanup
    {
        println!("\n--- Example 1: Basic usage with automatic cleanup ---");
        let rcu = Rcu::new(1);
        
        // Make some updates
        for i in 2..5 {
            let mut guard = rcu.assign_pointer().unwrap();
            *guard = i;
            println!("Updated value to: {}", *guard);
        }
        
        // When rcu goes out of scope, Drop will automatically try to clean up
        println!("Final value before drop: {}", *rcu);
    }
    println!("RCU instance dropped and cleanup attempted automatically");
    
    // Example 2: Long-lived clones with explicit cleanup
    println!("\n--- Example 2: Long-lived clones with explicit cleanup ---");
    let rcu = Rcu::new(10);
    
    // Create threads that demonstrate proper RCU usage
    let reader = thread::spawn({
        let rcu = rcu.read_lock();
        move || {
            println!("Reader thread started");
            for _ in 0..5 {
                // Create a short-lived clone for reading
                let value = *rcu;
                println!("Reader: {}", value);
                thread::sleep(Duration::from_millis(500));
            }
            println!("Reader thread finished");
        }
    });

    let writer = thread::spawn({
        let mut rcu = rcu.read_lock();
        move || {
            println!("Writer thread started");
            for i in 1..6 {
                {
                    let mut rcu_guard = match rcu.assign_pointer() {
                        Ok(guard) => guard,
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            return;
                        }
                    };
                    *rcu_guard += i;
                    println!("Writer: updated to {}", *rcu_guard);
                }
                
                // IMPORTANT: Explicitly clean up after updates
                // This is necessary for long-lived clones
                let cleaned = rcu.try_clean_fast();
                println!("Cleanup performed: {}", cleaned);
                
                thread::sleep(Duration::from_millis(1000));
            }
            println!("Writer thread finished");
        }
    });

    reader.join().unwrap();
    writer.join().unwrap();
    
    println!("\nAll threads completed. Final value: {}", *rcu);
    println!("=== Demo completed ===");
}
