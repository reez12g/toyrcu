mod rcu;

use rcu::Rcu;
use std::{thread, time::{Duration, Instant}};
use std::sync::Arc;

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

    // Example 3: Performance benchmark
    benchmark_rcu_performance();

    println!("=== Demo completed ===");
}

fn benchmark_rcu_performance() {
    println!("\n--- Example 3: Performance Benchmark ---");

    // Simple benchmark of optimized RCU
    let start_time = Instant::now();

    // Benchmark reads
    let read_rcu = Rcu::new(42i32);
    let read_start = Instant::now();
    const READ_ITERATIONS: usize = 1_000_000;

    // Perform a large number of reads
    let mut sum = 0i32;
    for _ in 0..READ_ITERATIONS {
        let value: i32 = *read_rcu;
        sum += value;
    }

    let read_time = read_start.elapsed();
    println!("Completed {} reads in {:.3} seconds", READ_ITERATIONS, read_time.as_secs_f64());
    println!("Read throughput: {:.2} million ops/sec",
             (READ_ITERATIONS as f64) / read_time.as_secs_f64() / 1_000_000.0);

    // Benchmark write + cleanup operations
    let mut write_rcu = Rcu::new(0i32);
    let write_start = Instant::now();
    const WRITE_ITERATIONS: usize = 100_000;

    // Perform a number of writes with periodic cleanup
    for i in 0..WRITE_ITERATIONS {
        // Update the value
        let mut guard = write_rcu.assign_pointer().unwrap();
        *guard = i as i32;

        // Periodically clean up
        if i % 10 == 0 {
            drop(guard);  // Must drop guard before cleaning
            write_rcu.try_clean_fast();
        }
    }

    let write_time = write_start.elapsed();
    println!("Completed {} writes in {:.3} seconds", WRITE_ITERATIONS, write_time.as_secs_f64());
    println!("Write throughput: {:.2} million ops/sec",
             (WRITE_ITERATIONS as f64) / write_time.as_secs_f64() / 1_000_000.0);

    // Overall benchmark stats
    let total_time = start_time.elapsed();
    let total_ops = READ_ITERATIONS + WRITE_ITERATIONS;
    println!("Overall: {:.2} million operations per second",
             (total_ops as f64) / total_time.as_secs_f64() / 1_000_000.0);
}
