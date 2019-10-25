use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

type BoxFut = Box<dyn Future<Output = ()> + Send + 'static>;

#[derive(Clone)]
struct Task(Arc<TaskInner>);

impl Task {
    fn into_raw(this: Task) -> *const TaskInner {
        Arc::into_raw(this.0)
    }

    unsafe fn from_raw(ptr: *const TaskInner) -> Self {
        Self(Arc::from_raw(ptr))
    }

    fn wake(self) {
        if let Some(queue) = self.0.queue.upgrade() {
            queue.queue(self);
        }
    }

    fn into_raw_waker(this: Task) -> std::task::RawWaker {
        std::task::RawWaker::new(
            Task::into_raw(this) as *const (),
            &std::task::RawWakerVTable::new(
                waker_clone_fn,
                waker_wake_fn,
                waker_wake_by_ref_fn,
                waker_drop_fn,
            ),
        )
    }
}

struct TaskInner {
    future: Mutex<Option<BoxFut>>,
    queue: Weak<TaskQueue>,
}

struct TaskQueue {
    queue: Mutex<VecDeque<Task>>,
    queue_cv: Condvar,
}

impl TaskQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(32)),
            queue_cv: Condvar::new(),
        }
    }

    fn new_task(self: Arc<TaskQueue>, future: BoxFut) {
        let task = Task(Arc::new(TaskInner {
            future: Mutex::new(Some(future)),
            queue: Arc::downgrade(&self),
        }));

        self.queue(task);
    }

    fn queue(&self, task: Task) {
        let mut queue = self.queue.lock().unwrap();
        queue.push_back(task);
        self.queue_cv.notify_one();
    }

    /// Blocks until a task is available
    fn get_task(&self) -> Task {
        let mut queue = self.queue.lock().unwrap();
        loop {
            if let Some(task) = queue.pop_front() {
                return task;
            } else {
                queue = self.queue_cv.wait(queue).unwrap();
            }
        }
    }
}

pub struct ThreadPool {
    _threads: Mutex<Vec<JoinHandle<()>>>,
    queue: Arc<TaskQueue>,
}

impl ThreadPool {
    pub fn new() -> io::Result<Self> {
        let count = 2; //num_cpus()?;

        let queue = Arc::new(TaskQueue::new());

        let mut threads = Vec::new();
        for thread_id in 0..count {
            threads.push(std::thread::spawn({
                let queue = Arc::clone(&queue);
                move || thread_main(queue, thread_id)
            }));
        }

        Ok(Self {
            _threads: Mutex::new(threads),
            queue,
        })
    }

    pub fn spawn_ok<T>(&self, future: T)
    where
        T: Future<Output = ()> + Send + 'static,
    {
        self.do_spawn(Box::new(future));
    }

    fn do_spawn(&self, future: BoxFut) {
        Arc::clone(&self.queue).new_task(future);
    }

    pub fn run<R, T>(&self, future: T) -> R
    where
        T: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let mutex: Arc<Mutex<Option<R>>> = Arc::new(Mutex::new(None));
        let cv = Arc::new(Condvar::new());
        let mut guard = mutex.lock().unwrap();
        self.spawn_ok({
            let mutex = Arc::clone(&mutex);
            let cv = Arc::clone(&cv);
            async move {
                let result = future.await;
                *(mutex.lock().unwrap()) = Some(result);
                cv.notify_all();
            }
        });
        loop {
            guard = cv.wait(guard).unwrap();
            if let Some(result) = guard.take() {
                return result;
            }
        }
    }
}

thread_local! {
    static CURRENT_QUEUE: RefCell<*const TaskQueue> = RefCell::new(std::ptr::null());
    static CURRENT_TASK: RefCell<*const Task> = RefCell::new(std::ptr::null());
}

fn thread_main(task_queue: Arc<TaskQueue>, _thread_id: usize) {
    CURRENT_QUEUE.with(|q| *q.borrow_mut() = task_queue.as_ref() as *const TaskQueue);

    let local_waker = unsafe {
        std::task::Waker::from_raw(std::task::RawWaker::new(
            std::ptr::null(),
            &std::task::RawWakerVTable::new(
                local_waker_clone_fn,
                local_waker_wake_fn,
                local_waker_wake_fn,
                local_waker_drop_fn,
            ),
        ))
    };

    let mut context = Context::from_waker(&local_waker);

    loop {
        let task: Task = task_queue.get_task();
        let task: Pin<&Task> = Pin::new(&task);
        let task = task.get_ref();
        CURRENT_TASK.with(|c| *c.borrow_mut() = task as *const Task);

        let mut task_future = task.0.future.lock().unwrap();
        match task_future.take() {
            Some(mut future) => {
                let pin = unsafe { Pin::new_unchecked(&mut *future) };
                match pin.poll(&mut context) {
                    Poll::Ready(()) => (), // done with that task
                    Poll::Pending => {
                        *task_future = Some(future);
                    }
                }
            }
            None => eprintln!("task polled after ready"),
        }
    }
}

unsafe fn local_waker_clone_fn(_: *const ()) -> std::task::RawWaker {
    let task: Task = CURRENT_TASK.with(|t| Task::clone(&**t.borrow()));
    Task::into_raw_waker(task)
}

unsafe fn local_waker_wake_fn(_: *const ()) {
    let task: Task = CURRENT_TASK.with(|t| Task::clone(&**t.borrow()));
    CURRENT_QUEUE.with(|q| (**q.borrow()).queue(task));
}

unsafe fn local_waker_drop_fn(_: *const ()) {}

unsafe fn waker_clone_fn(this: *const ()) -> std::task::RawWaker {
    let this = Task::from_raw(this as *const TaskInner);
    let clone = this.clone();
    let _ = Task::into_raw(this);
    Task::into_raw_waker(clone)
}

unsafe fn waker_wake_fn(this: *const ()) {
    let this = Task::from_raw(this as *const TaskInner);
    this.wake();
}

unsafe fn waker_wake_by_ref_fn(this: *const ()) {
    let this = Task::from_raw(this as *const TaskInner);
    this.clone().wake();
    let _ = Task::into_raw(this);
}

unsafe fn waker_drop_fn(this: *const ()) {
    let _this = Task::from_raw(this as *const TaskInner);
}

pub fn num_cpus() -> io::Result<usize> {
    let rc = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc as usize)
    }
}
