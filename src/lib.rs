#![feature(shared)]
#![feature(rustc_private)]

#[macro_use]
extern crate log;

use std::ops::Deref;
use std::ptr::Shared;
use std::sync::atomic;
use std::sync::atomic::Ordering::{Release, Acquire, AcqRel};

#[derive(Debug)]
struct RcuInner<T> {
    refs: atomic::AtomicUsize,
    data: T,
}

unsafe impl<T: Sync + Send> Send for RcuInner<T> {}
unsafe impl<T: Sync + Send> Sync for RcuInner<T> {}


struct Link<T> {
    ptr: atomic::AtomicPtr<T>,
}

pub struct RcuCell<T> {
    link: Link<RcuInner<T>>,
}

unsafe impl<T: Sync + Send> Send for RcuCell<T> {}
unsafe impl<T: Sync + Send> Sync for RcuCell<T> {}

pub struct RcuReader<T> {
    inner: Shared<RcuInner<T>>,
}

unsafe impl<T: Sync + Send> Send for RcuReader<T> {}
unsafe impl<T: Sync + Send> Sync for RcuReader<T> {}

impl<T> Deref for Link<T> {
    type Target = atomic::AtomicPtr<T>;

    #[inline]
    fn deref(&self) -> &atomic::AtomicPtr<T> {
        &self.ptr
    }
}

impl<T> Link<RcuInner<T>> {
    #[inline]
    fn get(&self) -> RcuReader<T> {
        let ptr = self.load(Acquire);
        unsafe {
            (*ptr).add_ref();
            RcuReader { inner: Shared::new(ptr) }
        }
    }
}

impl<T> RcuInner<T> {
    #[inline]
    fn new(data: T) -> Self {
        RcuInner {
            refs: atomic::AtomicUsize::new(1),
            data: data,
        }

    }

    #[inline]
    fn add_ref(&self) {
        self.refs.fetch_add(1, Release);
    }

    #[inline]
    fn release(&self) -> usize {
        let ret = self.refs.fetch_sub(1, Release);
        // to prevent delete data unsyned
        error!("{:?}", ret - 1);
        atomic::fence(Acquire);
        ret - 1
    }
}

impl<T> Drop for RcuReader<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if (**self.inner).release() == 0 {
                error!("---------------------released");
                // drop the inner box
                let _: Box<RcuInner<T>> = Box::from_raw(*self.inner);
            }
        }
    }
}

impl<T> Deref for RcuReader<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &(**self.inner).data }
    }
}

impl<T> RcuReader<T> {
    #[inline]
    fn unlink(&self) {
        unsafe {
            (**self.inner).release();
        }
    }
}

impl<T> RcuCell<T> {
    pub fn new(data: T) -> Self {
        let data = Box::new(RcuInner::new(data));
        RcuCell { link: Link { ptr: atomic::AtomicPtr::new(Box::into_raw(data)) } }
    }

    pub fn read(&self) -> RcuReader<T> {
        self.link.get()
    }

    pub fn update(&self, data: T) {
        let data = Box::new(RcuInner::new(data));
        let old = self.link.swap(Box::into_raw(data), AcqRel);

        // release the old data
        unsafe {
            (*old).add_ref();
            let d = RcuReader { inner: Shared::new(old) };
            d.unlink();
        }
    }
}

impl<T> Drop for RcuCell<T> {
    #[inline]
    fn drop(&mut self) {
        let d = self.link.get();
        d.unlink();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn single_thread_test() {
        let t = RcuCell::new(10);
        assert!(*t.read() == 10);
        t.update(5);
        assert!(*t.read() == 5);
    }
}