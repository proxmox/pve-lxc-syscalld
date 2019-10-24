use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering};

// We only perform a handful of memory read/writes in push()/pop(), so we use spin locks for
// performance reasons:

struct SpinLock(AtomicBool);
struct SpinLockGuard<'a>(&'a AtomicBool);

impl SpinLock {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    fn lock(&self) -> SpinLockGuard {
        while self.0.compare_and_swap(false, true, Ordering::Acquire) {
            // spin
        }
        SpinLockGuard(&self.0)
    }
}

impl Drop for SpinLockGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct Ring<T> {
    head: usize,
    tail: usize,
    mask: usize,
    data: Box<[MaybeUninit<T>]>,
    push_lock: SpinLock,
    pop_lock: SpinLock,
}

impl<T> Ring<T> {
    pub fn new(size: usize) -> Self {
        if size < 2 || size.count_ones() != 1 {
            panic!("Ring size must be a power of two!");
        }

        let mut data = Vec::with_capacity(size);
        unsafe {
            data.set_len(size);
        }

        Self {
            head: 0,
            tail: 0,
            mask: size - 1,
            data: data.into_boxed_slice(),
            push_lock: SpinLock::new(),
            pop_lock: SpinLock::new(),
        }
    }

    pub fn len(&self) -> usize {
        fence(Ordering::Acquire);
        self.tail - self.head
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
        let _guard = self.push_lock.lock();

        let tail = self.atomic_tail().load(Ordering::Acquire);
        let head = self.head;

        if tail - head == self.data.len() {
            return false;
        }

        unsafe {
            ptr::write(self.data[tail & self.mask].as_ptr() as *mut T, data);
        }
        self.atomic_tail().store(tail + 1, Ordering::Release);

        true
    }

    pub fn try_pop(&self) -> Option<T> {
        let _guard = self.pop_lock.lock();

        let head = self.atomic_head().load(Ordering::Acquire);
        let tail = self.tail;

        if tail - head == 0 {
            return None;
        }

        let data = unsafe { ptr::read(self.data[head & self.mask].as_ptr()) };

        self.atomic_head().store(head + 1, Ordering::Release);

        Some(data)
    }
}
