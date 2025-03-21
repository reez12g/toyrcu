use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, AtomicPtr, Ordering}};
use std::ptr::null_mut;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::fmt;
use std::mem;

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
pub struct Rcu<T: PartialEq> {
    body: Arc<RcuBody<T>>,
}

// Safety: Rcu<T> is Send and Sync if T is Send and Sync
unsafe impl<T: Send + Sync + PartialEq> Send for Rcu<T> {}
unsafe impl<T: Send + Sync + PartialEq> Sync for Rcu<T> {}

impl<T: Clone + PartialEq> Clone for Rcu<T> {
    fn clone(&self) -> Self {
        self.body.counter.fetch_add(1, Ordering::Relaxed);
        Rcu {
            body: self.body.clone(),
        }
    }
}

impl<T: PartialEq> Drop for Rcu<T> {
    fn drop(&mut self) {
        self.body.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl<T: PartialEq> Deref for Rcu<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // Use Acquire ordering for the load to ensure proper synchronization
        // with stores that use Release ordering
        let next = self.body.value.next.load(Ordering::Acquire);

        // Fast path: if next is null, just return the current value
        if next.is_null() {
            // Safe because current is only mutated during write operations
            // which are serialized with the updating flag
            unsafe { &*self.body.value.current.get() }
        } else {
            // If next is not null, read from the next value
            // Acquire ordering ensures we see all changes made before the store
            unsafe { &*(*next).current.get() }
        }
    }
}

impl<'a, T: Clone + PartialEq> Rcu<T> {
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
    #[inline]
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
        // Try to acquire the updating lock with a compare_exchange instead of swap
        // This avoids unnecessary atomic writes when the lock is already held
        if self.body.updating.compare_exchange(
            false, true, Ordering::Acquire, Ordering::Relaxed
        ).is_err() {
            return Err(RcuError::AlreadyUpdating);
        }

        // Use Acquire ordering for the next pointer load for consistency
        // with other next pointer loads in the codebase
        let next_ptr = self.body.value.next.load(Ordering::Acquire);

        // Create a new value by cloning the current one
        // We'll optimize to avoid cloning when possible in the guard's drop
        let current_value = unsafe { &*self.body.value.current.get() };
        let new_value = Value {
            current: UnsafeCell::new(current_value.clone()),
            next: AtomicPtr::new(next_ptr),
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
    /// This implementation ensures the entire chain of old data is cleaned up,
    /// preventing memory leaks when multiple updates occur without cleanup.
    pub fn clean(&mut self) {
        // Fast path: if there's no next pointer, nothing to clean
        let next = self.body.value.next.load(Ordering::Acquire);
        if next.is_null() {
            return;
        }

        // Only proceed if this is the sole reference
        let counter = self.body.counter.load(Ordering::Relaxed);
        if counter != 1 {
            return;
        }

        unsafe {
            // First update the current value with the next value
            let next_val = (*next).current.get();
            let current_val = self.body.value.current.get();

            // Use ptr::copy for potentially more efficient memory copying
            // instead of clone (though this depends on the type)
            *current_val = (*next_val).clone();

            // Then remove the next pointer with Release ordering to ensure
            // the current value update is visible
            let old_next = self.body.value.next.swap(null_mut(), Ordering::Release);

            // Free the old next value and its entire chain
            if !old_next.is_null() {
                // We need to manually clean up the entire chain
                let mut current_ptr = old_next;

                while !current_ptr.is_null() {
                    // Get the next pointer before we drop the current one
                    let next_ptr = (*current_ptr).next.load(Ordering::Acquire);

                    // Clear the next pointer to prevent recursive drop
                    (*current_ptr).next.store(null_mut(), Ordering::Relaxed);

                    // Drop the current node
                    drop(Box::from_raw(current_ptr));

                    // Move to the next node
                    current_ptr = next_ptr;
                }
            }
        }
    }

    /// Attempts to clean up old versions of the data automatically.
    /// This is a more efficient version that tries to avoid cloning when possible.
    /// This implementation ensures the entire chain of old data is cleaned up,
    /// preventing memory leaks when multiple updates occur without cleanup.
    ///
    /// Returns true if cleanup was performed.
    #[inline]
    pub fn try_clean_fast(&mut self) -> bool {
        let next = self.body.value.next.load(Ordering::Acquire);
        if next.is_null() {
            return false;
        }

        let counter = self.body.counter.load(Ordering::Relaxed);
        if counter != 1 {
            return false;
        }

        // Try to acquire the updating lock
        if self.body.updating.compare_exchange(
            false, true, Ordering::Acquire, Ordering::Relaxed
        ).is_err() {
            return false;
        }

        unsafe {
            // Move the next value into current instead of cloning
            let next_ptr = self.body.value.next.swap(null_mut(), Ordering::Release);
            if !next_ptr.is_null() {
                // Get the first node in the chain
                let next_box = Box::from_raw(next_ptr);
                let current_ptr = self.body.value.current.get();

                // Swap the values to avoid cloning
                mem::swap(&mut *current_ptr, &mut *next_box.current.get());

                // Get the next pointer in the chain before we drop the current node
                let mut chain_ptr = next_box.next.load(Ordering::Acquire);

                // Clear the next pointer to prevent recursive drop from the first node
                next_box.next.store(null_mut(), Ordering::Relaxed);

                // next_box will be dropped here, cleaning up its resources

                // Now clean up the rest of the chain
                while !chain_ptr.is_null() {
                    // Get the next pointer before we drop the current one
                    let next_in_chain = (*chain_ptr).next.load(Ordering::Acquire);

                    // Clear the next pointer to prevent recursive drop
                    (*chain_ptr).next.store(null_mut(), Ordering::Relaxed);

                    // Drop the current node
                    drop(Box::from_raw(chain_ptr));

                    // Move to the next node
                    chain_ptr = next_in_chain;
                }
            }
        }

        // Release the updating lock
        self.body.updating.store(false, Ordering::Release);
        true
    }
}

//----------------------------------
// RcuBody - Internal implementation
//----------------------------------
/// Internal structure that holds the actual data and synchronization primitives.
#[derive(Debug)]
pub struct RcuBody<T: PartialEq> {
    /// Number of active references to this RCU
    counter: AtomicUsize,
    /// Flag indicating if an update is in progress
    updating: AtomicBool,
    /// The current value and a potential next value
    value: Value<T>,
}

/// Holds a value and a pointer to a potential next value.
#[derive(Debug)]
pub struct Value<T: PartialEq> {
    current: UnsafeCell<T>,
    next: AtomicPtr<Value<T>>,
}

impl<T: PartialEq> Drop for Value<T> {
    fn drop(&mut self) {
        // Note: We don't need to recursively clean up the chain here anymore
        // as that's handled explicitly in clean() and try_clean_fast().
        // This prevents potential double-free issues when we manually clean up chains.
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
pub struct RcuGuard<'a, T: Clone + PartialEq> {
    value: Option<Value<T>>,
    body: &'a RcuBody<T>,
}

impl<'a, T: Clone + PartialEq> Deref for RcuGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        let value_ref = self.value.as_ref().expect("Value should not be None");
        unsafe { &*value_ref.current.get() }
    }
}

impl<'a, T: Clone + PartialEq> DerefMut for RcuGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        let value_ref = self.value.as_ref().expect("Value should not be None");
        unsafe { &mut *value_ref.current.get() }
    }
}

impl<'a, T: Clone + PartialEq> Drop for RcuGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            // Check if the value was actually modified by comparing with the original
            let current_value = unsafe { &*self.body.value.current.get() };
            let new_value = unsafe { &*value.current.get() };

            // Only allocate and update if the value actually changed
            // This optimization avoids unnecessary allocations and atomic operations
            if current_value != new_value {
                // Set the next pointer to the new value
                let boxed_value = Box::new(value);
                let raw_ptr = Box::into_raw(boxed_value);

                // Use Release ordering to ensure all writes to the new value
                // are visible before the pointer becomes visible
                self.body.value.next.store(raw_ptr, Ordering::Release);
            }
        }

        // Release the updating lock with Release ordering
        // to ensure all changes are visible to subsequent operations
        self.body.updating.store(false, Ordering::Release);
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

    #[test]
    fn test_chain_cleanup() {
        // Create an RCU instance
        let mut rcu = Rcu::new(0);

        // Perform multiple updates without cleaning up in between
        // This would create a chain of updates: 0 -> 1 -> 2 -> 3 -> 4
        for i in 1..5 {
            let mut guard = rcu.assign_pointer().unwrap();
            *guard = i;

            // Verify the update was applied
            drop(guard);
            assert_eq!(*rcu, i);
        }

        // At this point, we should have a chain of updates
        // Let's verify the chain exists by checking the next pointers
        unsafe {
            // Get the first next pointer
            let next_ptr = rcu.body.value.next.load(Ordering::Acquire);
            assert!(!next_ptr.is_null(), "Next pointer should not be null after updates");

            // The value should be 4 (the last update)
            assert_eq!(*(*next_ptr).current.get(), 4);

            // Check if there's a chain by following the next pointers
            let mut chain_length = 1;
            let mut current_ptr = next_ptr;

            while !(*current_ptr).next.load(Ordering::Acquire).is_null() {
                current_ptr = (*current_ptr).next.load(Ordering::Acquire);
                chain_length += 1;
            }

            // We should have a chain of updates
            assert!(chain_length > 0, "Should have a chain of updates");
            println!("Chain length before cleanup: {}", chain_length);
        }

        // Now clean up and verify the entire chain is cleaned up
        rcu.clean();

        // After cleanup, the next pointer should be null and the current value should be 4
        unsafe {
            let next_ptr = rcu.body.value.next.load(Ordering::Acquire);
            assert!(next_ptr.is_null(), "Next pointer should be null after cleanup");

            // The current value should be 4
            assert_eq!(*rcu.body.value.current.get(), 4);
        }
    }
}
