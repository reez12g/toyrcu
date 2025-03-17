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

## Implementation

This implementation provides:
- Lock-free read access
- Copy-on-write updates
- Safe memory reclamation
- Thread-safe operation

## Usage

```rust
use toyrcu::Rcu;
use std::{thread, time::Duration};

fn main() {
    // Create a new RCU with an initial value
    let rcu = Rcu::new(1);

    // Reader thread example
    let reader = thread::spawn({
        let rcu = rcu.read_lock();
        move || {
            for _ in 0..5 {
                let value = rcu.read_lock();
                println!("Reader: {:?}", *value);
                thread::sleep(Duration::from_millis(500));
            }
        }
    });

    // Writer thread example
    let writer = thread::spawn({
        let mut rcu = rcu.read_lock();
        move || {
            for i in 0..5 {
                {
                    let mut rcu_guard = match rcu.assign_pointer() {
                        Ok(guard) => guard,
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            return;
                        }
                    };
                    *rcu_guard += i;
                    println!("Writer: {:?}", *rcu_guard);
                }
                rcu.clean();
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

## Resources for Learning About RCU

- [RCU Fundamentals](https://lwn.net/Articles/262464/)
- [Wikipedia: Read-copy-update](https://en.wikipedia.org/wiki/Read-copy-update)
- [Paul McKenney's RCU documentation](https://www.kernel.org/doc/Documentation/RCU/)
