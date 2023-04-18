mod rcu;

use rcu::Rcu;
use std::{thread, time::Duration};

fn main() {
    let rcu = Rcu::new(1);

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

    let writer = thread::spawn({
        let mut rcu = rcu.read_lock();
        move || {
            for i in 0..5 {
                {
                    let mut rcu_guard = match rcu.assign_pointer() {
                        Some(guard) => guard,
                        None => {
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
