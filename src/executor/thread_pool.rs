use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

use super::num_cpus;
use super::ring::Ring;
use super::slot_list::SlotList;

type BoxFut = Box<dyn Future<Output = ()> + Send + 'static>;
type TaskId = usize;

struct Task {
    id: TaskId,
    pool: Arc<ThreadPool>,
    future: Option<(BoxFut, std::task::Waker)>,
}

pub struct ThreadPool {
    inner: Arc<ThreadPoolInner>,
}

impl ThreadPool {
    pub fn new() -> io::Result<Self> {
        let count = num_cpus()?;

        let inner = Arc::new(ThreadPoolInner {
            threads: Mutex::new(Vec::new()),
            tasks: RwLock::new(SlotList::new()),
            overflow: RwLock::new(Vec::new()),
        });

        let mut threads = inner.threads.lock().unwrap();
        for thread_id in 0..count {
            threads.push(Thread::new(Arc::clone(&inner), thread_id));
        }
        drop(threads);

        Ok(ThreadPool { inner })
    }

    pub fn spawn<T>(&self, future: T)
    where
        T: Future<Output = ()> + Send + 'static,
    {
        self.inner.spawn(Box::new(future))
    }
}

struct ThreadPoolInner {
    threads: Mutex<Vec<Thread>>,
    tasks: RwLock<SlotList<BoxFut>>,
    overflow: RwLock<Vec<TaskId>>,
}

unsafe impl Sync for ThreadPoolInner {}

impl ThreadPoolInner {
    fn create_task(&self, future: BoxFut) -> TaskId {
        self.tasks.write().unwrap().add(future)
    }

    fn spawn(&self, future: BoxFut) {
        self.queue_task(self.create_task(future))
    }

    fn queue_task(&self, task: TaskId) {
        let threads = self.threads.lock().unwrap();

        let shortest = threads
            .iter()
            .min_by(|a, b| a.task_count().cmp(&b.task_count()))
            .expect("thread pool should not be empty");

        if !shortest.try_queue(task) {
            drop(threads);
            self.overflow.write().unwrap().push(task);
        }
    }

    fn create_waker(self: Arc<Self>, task_id: TaskId) -> std::task::RawWaker {
        let waker = Box::new(Waker {
            pool: self,
            task_id,
        });
        std::task::RawWaker::new(Box::leak(waker) as *mut Waker as *mut (), &WAKER_VTABLE)
    }
}

struct Thread {
    handle: JoinHandle<()>,
    inner: Arc<ThreadInner>,
}

impl Thread {
    fn new(pool: Arc<ThreadPoolInner>, id: usize) -> Self {
        let inner = Arc::new(ThreadInner {
            id,
            ring: Ring::new(32),
            pool,
        });

        let handle = std::thread::spawn({
            let inner = Arc::clone(&inner);
            move || inner.thread_main()
        });
        Thread { handle, inner }
    }

    fn task_count(&self) -> usize {
        self.inner.task_count()
    }

    fn try_queue(&self, task: TaskId) -> bool {
        self.inner.try_queue(task)
    }
}

struct ThreadInner {
    id: usize,
    ring: Ring<TaskId>,
    pool: Arc<ThreadPoolInner>,
}

thread_local! {
    static THREAD_INNER: RefCell<*const ThreadInner> = RefCell::new(std::ptr::null());
}

impl ThreadInner {
    fn thread_main(self: Arc<Self>) {
        THREAD_INNER.with(|inner| {
            *inner.borrow_mut() = self.as_ref() as *const Self;
        });
        loop {
            if let Some(task_id) = self.ring.try_pop() {
                self.poll_task(task_id);
            }
        }
    }

    fn poll_task(&self, task_id: TaskId) {
        //let future = {
        //    let task = self.pool.tasks.read().unwrap().get(task_id).unwrap();
        //    if let Some(future) = task.future.as_ref() {
        //        future.as_ref() as *const (dyn Future<Output = ()> + Send + 'static)
        //            as *mut (dyn Future<Output = ()> + Send + 'static)
        //    } else {
        //        return;
        //    }
        //};
        let waker = unsafe {
            std::task::Waker::from_raw(std::task::RawWaker::new(
                task_id as *const (),
                &std::task::RawWakerVTable::new(
                    local_waker_clone_fn,
                    local_waker_wake_fn,
                    local_waker_wake_by_ref_fn,
                    local_waker_drop_fn,
                ),
            ))
        };

        let mut context = Context::from_waker(&waker);

        let future = {
            self.pool
                .tasks
                .read()
                .unwrap()
                .get(task_id)
                .unwrap()
                .as_ref() as *const (dyn Future<Output = ()> + Send + 'static)
                as *mut (dyn Future<Output = ()> + Send + 'static)
        };
        if let Poll::Ready(value) = unsafe { Pin::new_unchecked(&mut *future) }.poll(&mut context) {
            let task = self.pool.tasks.write().unwrap().remove(task_id);
        }
    }

    fn task_count(&self) -> usize {
        self.ring.len()
    }

    fn try_queue(&self, task: TaskId) -> bool {
        self.ring.try_push(task)
    }
}

struct RefWaker<'a> {
    pool: &'a ThreadPoolInner,
    task_id: TaskId,
}

struct Waker {
    pool: Arc<ThreadPoolInner>,
    task_id: TaskId,
}

pub struct Work {}

const WAKER_VTABLE: std::task::RawWakerVTable = std::task::RawWakerVTable::new(
    waker_clone_fn,
    waker_wake_fn,
    waker_wake_by_ref_fn,
    waker_drop_fn,
);

unsafe fn waker_clone_fn(_this: *const ()) -> std::task::RawWaker {
    panic!("TODO");
}

unsafe fn waker_wake_fn(_this: *const ()) {
    panic!("TODO");
}

unsafe fn waker_wake_by_ref_fn(_this: *const ()) {
    panic!("TODO");
}

unsafe fn waker_drop_fn(_this: *const ()) {
    panic!("TODO");
}

unsafe fn local_waker_clone_fn(_this: *const ()) -> std::task::RawWaker {
    panic!("TODO");
}

unsafe fn local_waker_wake_fn(_this: *const ()) {
    panic!("TODO");
}

unsafe fn local_waker_wake_by_ref_fn(_this: *const ()) {
    panic!("TODO");
}

unsafe fn local_waker_drop_fn(_this: *const ()) {
    panic!("TODO");
}
