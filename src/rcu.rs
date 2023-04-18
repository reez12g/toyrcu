use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, AtomicPtr, Ordering}};
use std::ptr::null_mut;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};

//----------------------------------
// Rcu
//----------------------------------
pub struct Rcu<T> {
    body: Arc<RcuBody<T>>,
}

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
        self.body.counter.fetch_add(1, Ordering::Relaxed);
        let next = self.body.value.next.load(Ordering::Acquire);
        if next.is_null() {
            unsafe { &*self.body.value.current.get() }
        } else {
            unsafe { &*(*next).current.get() }
        }
    }
}

impl<T> std::borrow::Borrow<T> for Rcu<T> {
    fn borrow(&self) -> &T {
        &**self
    }
}

impl<'a, T: Clone> Rcu<T> {
    pub fn new(x: T) -> Self {
        Rcu {
            body: Arc::new(RcuBody {
                counter: AtomicUsize::new(0),
                updating: AtomicBool::new(false),
                value: Value {
                    current: UnsafeCell::new(x),
                    next: AtomicPtr::new(null_mut()),
                },
            }),
        }
    }

    pub fn read_lock(&self) -> Self {
        self.clone()
    }

    pub fn assign_pointer(&'a self) -> RcuGuard<'a, T> {
        if self.body.updating.swap(true, Ordering::Relaxed) {
            panic!("Cannont update an Rcu twice simultaneously.");
        }

        let new_value = Value {
            current: UnsafeCell::new((*(*self)).clone()),
            next: AtomicPtr::new(self.body.value.next.load(Ordering::Acquire)),
        };

        RcuGuard {
            value: Some(new_value),
            body: &self.body,
        }
    }

    pub fn clean(&mut self) {
        let counter = self.body.counter.load(Ordering::Relaxed);
        let next = self.body.value.next.load(Ordering::Acquire);
        if counter == 0 && !next.is_null() {
            unsafe {
                let buffer: UnsafeCell<Option<T>> = UnsafeCell::new(None);
                std::ptr::copy_nonoverlapping(
                    self.body.value.current.get(),
                    buffer.get() as *mut T,
                    1,
                );
                std::ptr::copy_nonoverlapping((*next).current.get(), self.body.value.current.get(), 1);
                let _to_be_freed =
                    Box::from_raw(self.body.value.next.swap(null_mut(), Ordering::Release));
                std::ptr::copy_nonoverlapping(buffer.get() as *mut T, (*next).current.get(), 1);
                let buffer_copy: UnsafeCell<Option<T>> = UnsafeCell::new(None);
                std::ptr::copy_nonoverlapping(buffer_copy.get(), buffer.get(), 1);
            }
        }
    }
}

//----------------------------------
// RcuBody
//----------------------------------
pub struct RcuBody<T> {
    counter: AtomicUsize,
    updating: AtomicBool,
    value: Value<T>,
}

pub struct Value<T> {
    current: UnsafeCell<T>,
    next: AtomicPtr<Value<T>>,
}

impl<T> Drop for Value<T> {
    fn drop(&mut self) {
        let next = self.next.load(Ordering::Acquire);
        if !next.is_null() {
            let _free_this = unsafe { Box::from_raw(next) };
        }
    }
}

//----------------------------------
// RcuGuard
//----------------------------------
pub struct RcuGuard<'a, T: Clone> {
    value: Option<Value<T>>,
    body: &'a RcuBody<T>,
}

impl<'a, T: Clone> Deref for RcuGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        if let Some(ref value) = self.value {
            unsafe { &*value.current.get() }
        } else {
            unreachable!()
        }
    }
}

impl<'a, T: Clone> DerefMut for RcuGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        if let Some(ref value) = self.value {
            unsafe { &mut *value.current.get() }
        } else {
            unreachable!()
        }
    }
}

impl<'a, T: Clone> Drop for RcuGuard<'a, T> {
    fn drop(&mut self) {
        let value = std::mem::replace(&mut self.value, None);
        self.body
            .value
            .next
            .store(Box::into_raw(Box::new(value.unwrap())), Ordering::Release);
        self.body.updating.store(false, Ordering::Relaxed);
    }
}
