use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct Ring<T> {
    head: usize,
    tail: usize,
    mask: usize,
    data: Box<[MaybeUninit<T>]>,
}

impl<T> Ring<T> {
    pub fn new(size: usize) -> Arc<Self> {
        if size < 2 || size.count_ones() != 1 {
            panic!("Ring size must be a power of two!");
        }

        let mut data = Vec::with_capacity(size);
        for _ in 0..size {
            data.push(MaybeUninit::uninit())
        }

        Arc::new(Self {
            head: 0,
            tail: 0,
            mask: size - 1,
            data: data.into_boxed_slice(),
        })
    }

    #[inline]
    fn atomic_tail(&self) -> &AtomicUsize {
        unsafe { &*(&self.tail as *const usize as *const AtomicUsize) }
    }

    #[inline]
    fn atomic_head(&self) -> &AtomicUsize {
        unsafe { &*(&self.head as *const usize as *const AtomicUsize) }
    }

    pub fn try_push(&self, data: T) -> bool {
        let head = self.head;
        let tail = self.atomic_tail().load(Ordering::Acquire);

        if tail - head == self.data.len() {
            return false;
        }

        unsafe {
            ptr::write(self.data[tail & self.mask].as_ptr() as *mut _, data);
        }

        self.atomic_tail().fetch_add(1, Ordering::Release);

        true
    }

    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail;
        let head = self.atomic_head().load(Ordering::Acquire);

        if tail - head == 0 {
            return None;
        }

        let data = unsafe { std::ptr::read(self.data[head & self.mask].as_ptr()) };

        self.atomic_head().fetch_add(1, Ordering::Release);

        Some(data)
    }
}
