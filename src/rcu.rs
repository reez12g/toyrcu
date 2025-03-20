use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, AtomicPtr, Ordering}};
use std::ptr::null_mut;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::fmt;

/// Error types for RCU operations
#[derive(Debug)]
pub enum RcuError {
    /// Cannot update RCU as it's currently being updated
    AlreadyUpdating,
}

impl fmt::Display for RcuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RcuError::AlreadyUpdating => write!(f, "RCU is already being updated"),
        }
    }
}

impl std::error::Error for RcuError {}

//----------------------------------
// Rcu - Read-Copy-Update mechanism
//----------------------------------
/// A concurrent data structure that allows readers to access data
/// without blocking writers and vice versa.
///
/// The `Rcu<T>` type provides a way to read data concurrently while updates
/// happen in the background, with updates becoming visible to new readers
/// once they complete.
#[derive(Debug)]
pub struct Rcu<T> {
    body: Arc<RcuBody<T>>,
}

// Safety: Rcu<T> is Send and Sync if T is Send and Sync
unsafe impl<T: Send + Sync> Send for Rcu<T> {}
unsafe impl<T: Send + Sync> Sync for Rcu<T> {}

impl<T: Clone> Clone for Rcu<T> {
    fn clone(&self) -> Self {
        self.body.counter.fetch_add(1, Ordering::Relaxed);
        Rcu {
            body: self.body.clone(),
        }
    }
}

impl<T> Drop for Rcu<T> {
    fn drop(&mut self) {
        self.body.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl<T> Deref for Rcu<T> {
    type Target = T;
    fn deref(&self) -> &T {
        let next = self.body.value.next.load(Ordering::Acquire);
        if next.is_null() {
            // Safe because current is only mutated during write operations
            // which are serialized with the updating flag
            unsafe { &*self.body.value.current.get() }
        } else {
            // If next is not null, read from the next value
            unsafe { &*(*next).current.get() }
        }
    }
}

impl<'a, T: Clone> Rcu<T> {
    /// Creates a new RCU instance with the given initial value.
    pub fn new(v: T) -> Self {
        Rcu {
            body: Arc::new(RcuBody {
                counter: AtomicUsize::new(1), // Start with 1 reference (the creator)
                updating: AtomicBool::new(false),
                value: Value {
                    current: UnsafeCell::new(v),
                    next: AtomicPtr::new(null_mut()),
                },
            }),
        }
    }

    /// Creates a read-locked reference to the RCU data.
    ///
    /// This increments the internal reference counter to prevent
    /// cleanup while references are still active.
    pub fn read_lock(&self) -> Self {
        self.clone()
    }

    /// Begins an update operation on the RCU data.
    ///
    /// Returns a guard that can be used to modify the data. When the guard
    /// is dropped, the changes become visible to new readers.
    ///
    /// # Errors
    ///
    /// Returns an error if the RCU is already being updated.
    pub fn assign_pointer(&'a self) -> Result<RcuGuard<'a, T>, RcuError> {
        if self.body.updating.swap(true, Ordering::Relaxed) {
            return Err(RcuError::AlreadyUpdating);
        }

        let new_value = Value {
            current: UnsafeCell::new((*(*self)).clone()),
            next: AtomicPtr::new(self.body.value.next.load(Ordering::Acquire)),
        };

        Ok(RcuGuard {
            value: Some(new_value),
            body: &self.body,
        })
    }

    /// Attempts to clean up old versions of the data.
    ///
    /// This should be called periodically when no readers are active
    /// to reclaim memory used by old versions.
    pub fn clean(&mut self) {
        let counter = self.body.counter.load(Ordering::Relaxed);
        let next = self.body.value.next.load(Ordering::Acquire);

        // Only clean if this is the sole reference and there's a next value
        if counter == 1 && !next.is_null() {
            unsafe {
                // First update the current value with the next value
                let next_val = (*next).current.get();
                let current_val = self.body.value.current.get();
                *current_val = (*next_val).clone();

                // Then remove the next pointer
                let _ = Box::from_raw(self.body.value.next.swap(null_mut(), Ordering::Release));
            }
        }
    }
}

//----------------------------------
// RcuBody - Internal implementation
//----------------------------------
/// Internal structure that holds the actual data and synchronization primitives.
#[derive(Debug)]
pub struct RcuBody<T> {
    /// Number of active references to this RCU
    counter: AtomicUsize,
    /// Flag indicating if an update is in progress
    updating: AtomicBool,
    /// The current value and a potential next value
    value: Value<T>,
}

/// Holds a value and a pointer to a potential next value.
#[derive(Debug)]
pub struct Value<T> {
    current: UnsafeCell<T>,
    next: AtomicPtr<Value<T>>,
}

impl<T> Drop for Value<T> {
    fn drop(&mut self) {
        let next = self.next.load(Ordering::Acquire);
        if !next.is_null() {
            // Free the memory of the next value
            unsafe { drop(Box::from_raw(next)) };
        }
    }
}

//----------------------------------
// RcuGuard - Write access guard
//----------------------------------
/// A guard that provides mutable access to the RCU data.
///
/// When the guard is dropped, the changes become visible to new readers.
#[derive(Debug)]
pub struct RcuGuard<'a, T: Clone> {
    value: Option<Value<T>>,
    body: &'a RcuBody<T>,
}

impl<'a, T: Clone> Deref for RcuGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        let value_ref = self.value.as_ref().expect("Value should not be None");
        unsafe { &*value_ref.current.get() }
    }
}

impl<'a, T: Clone> DerefMut for RcuGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        let value_ref = self.value.as_ref().expect("Value should not be None");
        unsafe { &mut *value_ref.current.get() }
    }
}

impl<'a, T: Clone> Drop for RcuGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            // Set the next pointer to the new value
            let boxed_value = Box::new(value);
            let raw_ptr = Box::into_raw(boxed_value);
            self.body.value.next.store(raw_ptr, Ordering::Release);
        }
        // Release the updating lock
        self.body.updating.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use std::sync::Arc;

    #[test]
    fn test_basic_functionality() {
        let rcu = Rcu::new(42);

        // Read value through pointer dereference
        unsafe {
            assert_eq!(*rcu.body.value.current.get(), 42);
        }

        // Update through guard
        {
            let mut guard = rcu.assign_pointer().unwrap();
            *guard = 100;
        }

        // Check value was updated
        unsafe {
            let next = rcu.body.value.next.load(Ordering::Acquire);
            assert!(!next.is_null());
            assert_eq!(*(*next).current.get(), 100);
        }
    }

    #[test]
    fn test_error_on_double_update() {
        let rcu = Rcu::new(0);

        // First update should succeed
        let guard = rcu.assign_pointer().unwrap();

        // Second update should fail with AlreadyUpdating error
        let result = rcu.assign_pointer();
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, RcuError::AlreadyUpdating));
        }

        // Drop the first guard
        drop(guard);

        // Now we should be able to update again
        assert!(rcu.assign_pointer().is_ok());
    }

    #[test]
    fn test_cleanup() {
        let mut rcu = Rcu::new(1);

        // Make some updates
        {
            let mut guard = rcu.assign_pointer().unwrap();
            *guard = 2;
        }

        {
            let mut guard = rcu.assign_pointer().unwrap();
            *guard = 3;
        }

        // Verify the value through deref
        unsafe {
            let next = rcu.body.value.next.load(Ordering::Acquire);
            assert!(!next.is_null());
            assert_eq!(*(*next).current.get(), 3);
        }

        // Clean up old versions
        rcu.clean();

        // Read the actual current value after cleanup
        unsafe {
            let val = *rcu.body.value.current.get();
            assert_eq!(val, 3, "Current value should be 3 after cleanup");
        }
    }

    #[test]
    fn test_multiple_readers() {
        let rcu = Arc::new(Rcu::new(0));

        // Start a writer that increments the value
        let writer_rcu = Arc::clone(&rcu);
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            let mut guard = writer_rcu.assign_pointer().unwrap();
            *guard = 42;
        });

        // Start multiple readers that read the value
        let mut readers = vec![];
        for _ in 0..5 {
            let reader_rcu = Arc::clone(&rcu);
            readers.push(thread::spawn(move || {
                // Sleep to ensure some reads happen before and some after the write
                thread::sleep(Duration::from_millis(100));

                // Read current value through direct access to verify it's valid
                unsafe {
                    let next = reader_rcu.body.value.next.load(Ordering::Acquire);
                    let val = if next.is_null() {
                        *reader_rcu.body.value.current.get()
                    } else {
                        *(*next).current.get()
                    };

                    // Value should be either 0 or 42
                    assert!(val == 0 || val == 42);
                }
            }));
        }

        // Wait for all threads to complete
        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }

        // Final value should be 42 - check through direct access
        unsafe {
            let next = rcu.body.value.next.load(Ordering::Acquire);
            let val = if next.is_null() {
                *rcu.body.value.current.get()
            } else {
                *(*next).current.get()
            };
            assert_eq!(val, 42);
        }
    }
}
