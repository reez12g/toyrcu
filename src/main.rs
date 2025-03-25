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

    // 1. Sequential Read Benchmark
    println!("\n1. Sequential Read Performance");
    let read_rcu = Rcu::new(42i32);
    let read_start = Instant::now();
    const READ_ITERATIONS: usize = 1_000_000;

    // Perform a large number of reads
    let mut _sum = 0i32;
    for _ in 0..READ_ITERATIONS {
        // Using normal Deref trait
        let value: &i32 = &read_rcu;
        _sum += *value;
    }

    let read_time = read_start.elapsed();
    println!("  Completed {} reads in {:.3} seconds", READ_ITERATIONS, read_time.as_secs_f64());
    println!("  Read throughput: {:.2} million ops/sec",
             (READ_ITERATIONS as f64) / read_time.as_secs_f64() / 1_000_000.0);

    // 2. Sequential Write Benchmark
    println!("\n2. Sequential Write Performance");
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
    println!("  Completed {} writes in {:.3} seconds", WRITE_ITERATIONS, write_time.as_secs_f64());
    println!("  Write throughput: {:.2} million ops/sec",
             (WRITE_ITERATIONS as f64) / write_time.as_secs_f64() / 1_000_000.0);

    // 3. Parallel Read Benchmark
    println!("\n3. Parallel Read Performance");
    const NUM_READER_THREADS: usize = 8;
    const PARALLEL_READ_ITERATIONS: usize = 500_000;

    let shared_rcu = Arc::new(Rcu::new(42i32));
    let parallel_read_start = Instant::now();
    let barrier = Arc::new(std::sync::Barrier::new(NUM_READER_THREADS + 1));

    let mut reader_handles = Vec::with_capacity(NUM_READER_THREADS);
    for thread_id in 0..NUM_READER_THREADS {
        let thread_rcu = Arc::clone(&shared_rcu);
        let thread_barrier = Arc::clone(&barrier);

        reader_handles.push(thread::spawn(move || {
            // Wait for all threads to be ready
            thread_barrier.wait();

            let mut local_sum = 0i32;
            for _ in 0..PARALLEL_READ_ITERATIONS {
                // Using type annotation to get value from Rcu
                let rcu_ref = &*thread_rcu;
                let val_ref: &i32 = &*rcu_ref;
                local_sum += *val_ref;
            }

            println!("  Reader thread {} completed {} reads", thread_id, PARALLEL_READ_ITERATIONS);
            local_sum
        }));
    }

    // Start all reader threads simultaneously
    barrier.wait();

    // Wait for all reader threads to complete
    let mut total_sum = 0;
    for handle in reader_handles {
        total_sum += handle.join().unwrap();
    }

    let parallel_read_time = parallel_read_start.elapsed();
    let total_parallel_reads = NUM_READER_THREADS * PARALLEL_READ_ITERATIONS;
    println!("  Completed {} total reads across {} threads in {:.3} seconds",
             total_parallel_reads, NUM_READER_THREADS, parallel_read_time.as_secs_f64());
    println!("  Parallel read throughput: {:.2} million ops/sec",
             (total_parallel_reads as f64) / parallel_read_time.as_secs_f64() / 1_000_000.0);
    println!("  Sum verification: {}", total_sum);

    // 4. Parallel Write Benchmark
    println!("\n4. Parallel Write Performance");
    const NUM_WRITER_THREADS: usize = 4;
    const PARALLEL_WRITE_ITERATIONS: usize = 10_000;

    let shared_write_rcu = Arc::new(Rcu::new(0i32));
    let parallel_write_start = Instant::now();
    let write_barrier = Arc::new(std::sync::Barrier::new(NUM_WRITER_THREADS + 1));

    let mut writer_handles = Vec::with_capacity(NUM_WRITER_THREADS);
    for _thread_id in 0..NUM_WRITER_THREADS {
        let thread_rcu = Arc::clone(&shared_write_rcu);
        let thread_barrier = Arc::clone(&write_barrier);

        writer_handles.push(thread::spawn(move || {
            // Wait for all threads to be ready
            thread_barrier.wait();

            let mut success_count = 0;
            let mut retry_count = 0;

            for _ in 0..PARALLEL_WRITE_ITERATIONS {
                // Try to update directly from thread_rcu
                let mut success = false;
                {
                    // Try to update
                    match thread_rcu.assign_pointer() {
                        Ok(mut guard) => {
                            // Access current value
                            let current = *guard;
                            // Update value
                            *guard = current + 1;
                            success = true;
                        },
                        Err(_) => {
                            retry_count += 1;
                        }
                    }
                }
                
                if success {
                    success_count += 1;
                }

                // Yield occasionally to let other threads run
                if success_count % 100 == 0 {
                    thread::yield_now();
                }
            }

            (success_count, retry_count)
        }));
    }

    // Start all threads
    write_barrier.wait();

    // Wait for completion
    let mut total_successes = 0;
    let mut total_retries = 0;
    for handle in writer_handles {
        let (successes, retries) = handle.join().unwrap();
        total_successes += successes;
        total_retries += retries;
    }

    let parallel_write_time = parallel_write_start.elapsed();
    println!("  Completed {} successful writes with {} retries across {} threads in {:.3} seconds",
             total_successes, total_retries, NUM_WRITER_THREADS, parallel_write_time.as_secs_f64());
    println!("  Parallel write throughput: {:.2} million ops/sec",
             (total_successes as f64) / parallel_write_time.as_secs_f64() / 1_000_000.0);

    // Get final value
    let final_write_value = {
        let value_ref: &i32 = &*shared_write_rcu;
        *value_ref
    };
    println!("  Final value: {}", final_write_value);

    // 5. Comparison of read/write operations
    println!("\n5. Performance Comparison");

    // Aggregate results from various benchmarks
    let seq_read_throughput = (READ_ITERATIONS as f64) / read_time.as_secs_f64() / 1_000_000.0;
    let seq_write_throughput = (WRITE_ITERATIONS as f64) / write_time.as_secs_f64() / 1_000_000.0;
    let par_read_throughput = (total_parallel_reads as f64) / parallel_read_time.as_secs_f64() / 1_000_000.0;
    let par_write_throughput = (total_successes as f64) / parallel_write_time.as_secs_f64() / 1_000_000.0;

    // Overall results
    let total_ops = READ_ITERATIONS + WRITE_ITERATIONS + total_parallel_reads + total_successes;
    let total_time = start_time.elapsed();
    let overall_throughput = (total_ops as f64) / total_time.as_secs_f64() / 1_000_000.0;

    println!("  Sequential read:  {:.2} million ops/sec", seq_read_throughput);
    println!("  Sequential write: {:.2} million ops/sec", seq_write_throughput);
    println!("  Parallel read:    {:.2} million ops/sec", par_read_throughput);
    println!("  Parallel write:   {:.2} million ops/sec", par_write_throughput);
    println!("  Read/write ratio: {:.2}x", seq_read_throughput / seq_write_throughput);
    println!("  Parallel speedup (read):  {:.2}x", par_read_throughput / seq_read_throughput);
    println!("  Parallel speedup (write): {:.2}x", par_write_throughput / seq_write_throughput);
    println!("  Overall:          {:.2} million operations per second", overall_throughput);
}
