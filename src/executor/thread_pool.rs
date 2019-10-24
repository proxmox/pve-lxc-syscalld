use std::future::Future;
use std::io;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use super::num_cpus;
use super::ring::Ring;
use super::slot_list::SlotList;

type BoxFut = Box<dyn Future<Output = ()> + Send + 'static>;
type TaskId = usize;

pub struct ThreadPool {
    inner: Arc<ThreadPoolInner>,
}

pub struct ThreadPoolInner {
    threads: Mutex<Vec<Thread>>,
    tasks: Mutex<SlotList<BoxFut>>,
}

pub struct Thread {
    handle: JoinHandle<()>,
    inner: Arc<ThreadInner>,
    queue_sender: mpsc::Sender<Work>,
}

pub struct ThreadInner {
    id: usize,
    ring: Ring<TaskId>,
}

pub struct Work {}

impl ThreadPool {
    pub fn new() -> io::Result<Self> {
        let count = num_cpus()?;

        let inner = Arc::new(ThreadPoolInner {
            threads: Mutex::new(Vec::new()),
            tasks: Mutex::new(SlotList::new()),
        });

        let mut threads = Vec::with_capacity(count);
        for thread_id in 0..count {
            threads.push(Thread::new(Arc::clone(&inner), thread_id));
        }

        *inner.threads.lock().unwrap() = threads;

        Ok(ThreadPool { inner })
    }

    pub fn spawn<T>(&self, future: T)
    where
        T: Future<Output = ()> + Send + 'static,
    {
        self.inner.spawn(Box::new(future))
    }
}

impl ThreadPoolInner {
    fn create_task(&self, future: BoxFut) -> TaskId {
        self.tasks.lock().unwrap().add(future)
    }

    fn spawn(&self, future: BoxFut) {
        self.queue_task(self.create_task(future))
    }

    fn queue_task(&self, task: TaskId) {
        let threads = self.threads.lock().unwrap();
        //let shortest = threads
        //    .iter()
        //    .min_by(|a, b| a.task_count().cmp(b.task_count()))
        //    .expect("thread pool should not be empty");
    }
}

impl Thread {
    fn new(pool: Arc<ThreadPoolInner>, id: usize) -> Self {
        let (queue_sender, queue_receiver) = mpsc::channel();

        let inner = Arc::new(ThreadInner {
            id,
            ring: Ring::new(32),
        });

        let handle = std::thread::spawn({
            let inner = Arc::clone(&inner);
            move || inner.thread_main(queue_receiver)
        });
        Thread {
            handle,
            inner,
            queue_sender,
        }
    }
}

impl ThreadInner {
    fn thread_main(self: Arc<Self>, queue: mpsc::Receiver<Work>) {
        let _ = queue;
    }
}
