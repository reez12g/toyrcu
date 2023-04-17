use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::ops::Deref;

pub struct Rcu<T> {
    body: Arc<RcuBody<T>>,
}

pub struct RcuBody<T> {
    data: T,
    counter: AtomicUsize,
}

impl<T> Rcu<T> {
    pub fn new(data: T) -> Self {
        Rcu {
            body: Arc::new(RcuBody {
                data,
                counter: AtomicUsize::new(1),
            }),
        }
    }
}

impl<T> RcuBody<T> {
    // ReaderがRCUで保護されたデータへのReadを開始することを示す。
    pub fn read_lock(&self) {
        self.counter.fetch_add(1, Ordering::Acquire);
    }

    // ReaderがCritical Sectionを抜けたことを示す。
    pub fn read_unlock(&self) {
        self.counter.fetch_sub(1, Ordering::Release);
    }

    // 全てのReaderがQuiescent Stateになるまで待機する。
    pub fn synchronize(&self) {
        while self.counter.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
    }

    // UpdaterがRCUで保護されたデータに新しい値を割り当てる。
    pub fn assign_pointer(&self, pointer: &AtomicUsize) {
    }

    // ReaderがRCUで保護されたデータ(ポインタ)を取得し、参照する。
    pub fn dereference(&self) -> &T {
        &self.data
    }
}

impl<T> Deref for Rcu<T> {
    type Target = RcuBody<T>;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}
