use std::sync::{Arc, atomic::{AtomicUsize, AtomicPtr, Ordering}};
use std::ptr::null_mut;
use std::cell::RefCell;
use std::ops::Deref;

pub struct Rcu<T> {
    body: Arc<RcuBody<T>>,
}

unsafe impl<T: Send + Sync> Send for Rcu<T> {}
unsafe impl<T: Send + Sync> Sync for Rcu<T> {}

impl<T: Clone> Clone for Rcu<T> {
    fn clone(&self) -> Self {
        Rcu {
            body: Arc::clone(&self.body),
        }
    }
}

impl<T> Deref for Rcu<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let next = self.body.next.load(Ordering::Acquire);
        if next == null_mut() {
            return self.body.data.borrow().deref();
        }
        unsafe { &*next.cast::<T>() }
    }
}

impl<'a, T: Clone> Rcu<T> {
    pub fn new(data: T) -> Self {
        Rcu {
            body: Arc::new(RcuBody {
                data: RefCell::new(data),
                next: AtomicPtr::new(null_mut()),
            }),
        }
    }

    // ReaderがRCUで保護されたデータへのReadを開始することを示す。
    pub fn read_lock(&self) -> Self {
        Rcu {
            body: Arc::clone(&self.body),
        }
    }

    // 全てのReaderがQuiescent Stateになるまで待機する。
    pub fn synchronize(&self) {
        core::hint::spin_loop();
    }

    // UpdaterがRCUで保護されたデータに新しい値を割り当てる。
    pub fn assign_pointer(&self, pointer: &AtomicUsize) {
    }

    // ReaderがRCUで保護されたデータ(ポインタ)を取得し、参照する。
    pub fn dereference(&self) -> &T {
        self.body.data.borrow().deref()
    }

}

pub struct RcuBody<T> {
    data: RefCell<T>,
    next: AtomicPtr<T>,
}
