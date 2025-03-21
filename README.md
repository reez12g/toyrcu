# toyrcu

toyrcu is a Read-Copy-Update (RCU) implementation written in Rust.
This project is meant for educational purposes and is not suitable for real-world production use.

## What is RCU?

Read-Copy-Update (RCU) is a synchronization mechanism that allows multiple readers to access data concurrently while a single writer updates it without blocking the readers. It's particularly useful in scenarios where reads are much more frequent than writes.

Key characteristics of RCU:
- Readers can access data without locks
- Writers make a copy of the data to modify
- Updates become visible to new readers once they complete
- Old versions are cleaned up when no readers are using them

## Memory Management

An important aspect of this RCU implementation is memory management:

- When you clone an `Rcu<T>` instance (or use `read_lock()`), old data is not automatically cleaned up until you explicitly call `clean()` or `try_clean_fast()` on a mutable reference.
- The `Drop` implementation will attempt to clean up automatically when an instance is dropped, but this is only possible when there are no other active references.
- For best results:
  - Keep clones short-lived when possible
  - Call `clean()` or `try_clean_fast()` periodically on long-lived clones
  - Be aware that failing to clean up can result in memory leaks as old versions of the data will be retained

## Implementation

This implementation provides:
- Lock-free read access
- Copy-on-write updates
- Safe memory reclamation
- Thread-safe operation

## Usage

### Basic Usage with Automatic Cleanup

For short-lived RCU instances, the automatic cleanup in the `Drop` implementation will handle memory management:

```rust
use toyrcu::Rcu;

fn main() {
    // Create a new RCU with an initial value
    let mut rcu = Rcu::new(1);

    // Make some updates
    for i in 2..5 {
        let mut guard = rcu.assign_pointer().unwrap();
        *guard = i;
        println!("Updated value to: {}", *guard);
    }

    // When rcu goes out of scope, Drop will automatically try to clean up
    println!("Final value before drop: {}", *rcu);
    // End of scope - automatic cleanup happens here
}
```

### Long-lived Clones with Explicit Cleanup

For long-lived clones, especially in multi-threaded contexts, explicit cleanup is necessary:

```rust
use toyrcu::Rcu;
use std::{thread, time::Duration};

fn main() {
    // Create a new RCU with an initial value
    let rcu = Rcu::new(10);

    // Reader thread example
    let reader = thread::spawn({
        let rcu = rcu.read_lock();
        move || {
            for _ in 0..5 {
                // Read the value (creates a short-lived reference)
                let value = *rcu;
                println!("Reader: {}", value);
                thread::sleep(Duration::from_millis(500));
            }
        }
    });

    // Writer thread example
    let writer = thread::spawn({
        let mut rcu = rcu.read_lock();
        move || {
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
                rcu.try_clean_fast();

                thread::sleep(Duration::from_millis(1000));
            }
        }
    });

    reader.join().unwrap();
    writer.join().unwrap();
}
```

## Building and Running

```
cargo build
cargo run
```

## Limitations

- This is a simplified implementation for learning purposes
- Not optimized for production workloads
- Lacks some features found in industrial-strength RCU implementations
- Requires manual memory management through `clean()` or `try_clean_fast()` for long-lived clones

## Resources for Learning About RCU

- [RCU Fundamentals](https://lwn.net/Articles/262464/)
- [Wikipedia: Read-copy-update](https://en.wikipedia.org/wiki/Read-copy-update)
- [Paul McKenney's RCU documentation](https://www.kernel.org/doc/Documentation/RCU/)
