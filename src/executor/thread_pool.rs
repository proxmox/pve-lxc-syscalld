use std::io;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::num_cpus;

pub struct ThreadPool {
    inner: Arc<Inner>,
}

pub struct Inner {
    threads: Mutex<Vec<Thread>>,
}

pub struct Thread {
    handle: JoinHandle<()>,
    id: usize,
}

impl ThreadPool {
    pub fn new() -> io::Result<Self> {
        let count = num_cpus()?;

        let inner = Arc::new(Inner {
            threads: Mutex::new(Vec::new()),
        });

        let mut threads = Vec::with_capacity(count);
        for thread_id in 0..count {
            threads.push(Thread::new(Arc::clone(&inner), thread_id));
        }

        *inner.threads.lock().unwrap() = threads;

        Ok(ThreadPool { inner })
    }
}

impl Thread {
    fn new(pool: Arc<Inner>, id: usize) -> Self {
        let handle = std::thread::spawn(move || Self::thread_main(pool, id));
        Self { handle, id }
    }

    fn thread_main(pool: Arc<Inner>, thread_id: usize) {
        let _ = pool;
        let _ = thread_id;
    }
}
