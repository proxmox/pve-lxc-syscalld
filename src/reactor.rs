use std::convert::TryFrom;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;

use crate::epoll::{Epoll, EpollEvent, EPOLLERR, EPOLLET, EPOLLHUP, EPOLLIN, EPOLLOUT};
use crate::error::io_err_other;
use crate::poll_fn::poll_fn;
use crate::tools::Fd;

static START: Once = Once::new();
static mut REACTOR: Option<Arc<Reactor>> = None;

pub fn default() -> Arc<Reactor> {
    START.call_once(|| unsafe {
        let reactor = Reactor::new().expect("setup main epoll reactor");
        REACTOR = Some(reactor);
    });
    unsafe { Arc::clone(REACTOR.as_ref().unwrap()) }
}

pub struct Reactor {
    epoll: Arc<Epoll>,
    removed: Mutex<Vec<Box<RegistrationInner>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Reactor {
    pub fn new() -> io::Result<Arc<Self>> {
        let epoll = Arc::new(Epoll::new()?);

        let this = Arc::new(Reactor {
            epoll,
            removed: Mutex::new(Vec::new()),
            thread: Mutex::new(None),
        });

        let handle = std::thread::spawn({
            let this = Arc::clone(&this);
            move || this.thread_main()
        });

        this.thread.lock().unwrap().replace(handle);

        Ok(this)
    }

    fn thread_main(self: Arc<Self>) {
        let mut buf: [EpollEvent; 16] = unsafe { std::mem::zeroed() };
        loop {
            let count = match self.epoll.wait(&mut buf, None) {
                Ok(count) => count,
                Err(err) => {
                    eprintln!("error in epoll loop: {}", err);
                    std::process::exit(1);
                }
            };
            for i in 0..count {
                self.handle_event(&buf[i]);
            }
            // After going through the events we can release memory associated with already closed
            // file descriptors:
            self.removed.lock().unwrap().clear();
        }
    }

    fn handle_event(&self, event: &EpollEvent) {
        let registration = unsafe { &mut *(event.r#u64 as *mut RegistrationInner) };
        if registration.gone.load(Ordering::Acquire) {
            return;
        }

        if 0 != (event.events & EPOLLIN) {
            //let _prev = registration.ready.fetch_or(READY_IN, Ordering::AcqRel);
            if let Some(waker) = registration.read_waker.lock().unwrap().take() {
                waker.wake();
            }
        }

        if 0 != (event.events & EPOLLOUT) {
            //let _prev = registration.ready.fetch_or(READY_OUT, Ordering::AcqRel);
            if let Some(waker) = registration.write_waker.lock().unwrap().take() {
                waker.wake();
            }
        }

        if 0 != (event.events & (EPOLLERR | EPOLLHUP)) {
            //let _prev = registration.ready.fetch_or(READY_ERR, Ordering::AcqRel);
            if let Some(waker) = registration.read_waker.lock().unwrap().take() {
                waker.wake();
            }
            if let Some(waker) = registration.write_waker.lock().unwrap().take() {
                waker.wake();
            }
        }
    }

    pub fn register(self: Arc<Self>, fd: RawFd) -> io::Result<Registration> {
        let mut inner = Box::new(RegistrationInner {
            gone: AtomicBool::new(false),
            reactor: Arc::clone(&self),
            read_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
        });

        let inner_ptr = {
            // type check/assertion
            let inner_ptr: &mut RegistrationInner = &mut *inner;
            // make raw pointer
            inner_ptr as *mut RegistrationInner as usize as u64
        };

        self.epoll
            .add_fd(fd, EPOLLIN | EPOLLOUT | EPOLLET, inner_ptr)?;

        Ok(Registration { inner: Some(inner) })
    }

    fn deregister(&self, registration: Box<RegistrationInner>) {
        self.removed.lock().unwrap().push(registration);
    }
}

pub struct Registration {
    // pin the data in memory because the other thread will access it
    // ManuallyDrop::take is nightly only :<
    inner: Option<Box<RegistrationInner>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let inner = self.inner.as_ref().unwrap();
        let reactor = Arc::clone(&inner.reactor);
        inner.gone.store(true, Ordering::Release);
        reactor.deregister(self.inner.take().unwrap());
    }
}

// This is accessed by the reactor
struct RegistrationInner {
    gone: AtomicBool,
    reactor: Arc<Reactor>,
    read_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
}

pub struct PolledFd {
    fd: Fd,
    registration: Registration,
}

impl PolledFd {
    pub fn new(fd: Fd) -> io::Result<Self> {
        Self::new_with_reactor(fd, crate::reactor::default())
    }

    pub fn new_with_reactor(fd: Fd, reactor: Arc<Reactor>) -> io::Result<Self> {
        let registration = reactor.register(fd.as_raw_fd())?;
        Ok(Self { fd, registration })
    }

    pub fn wrap_read<T, F>(&mut self, cx: &mut Context, func: F) -> Poll<io::Result<T>>
    where
        F: FnOnce() -> io::Result<T>,
    {
        let mut read_waker = self
            .registration
            .inner
            .as_ref()
            .unwrap()
            .read_waker
            .lock()
            .unwrap();
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                *read_waker = Some(cx.waker().clone());
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }

    pub fn wrap_write<T, F>(&mut self, cx: &mut Context, func: F) -> Poll<io::Result<T>>
    where
        F: FnOnce() -> io::Result<T>,
    {
        let mut write_waker = self
            .registration
            .inner
            .as_ref()
            .unwrap()
            .write_waker
            .lock()
            .unwrap();
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                *write_waker = Some(cx.waker().clone());
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl PolledFd {
    pub fn poll_read(&mut self, cx: &mut Context, data: &mut [u8]) -> Poll<io::Result<usize>> {
        let size = libc::size_t::try_from(data.len()).map_err(io_err_other)?;
        let fd = self.fd.as_raw_fd();
        self.wrap_read(cx, || {
            c_result!(unsafe { libc::read(fd, data.as_mut_ptr() as *mut libc::c_void, size) })
                .map(|res| res as usize)
        })
    }

    pub async fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        poll_fn(move |cx| self.poll_read(cx, data)).await
    }

    pub fn poll_write(&mut self, data: &[u8], cx: &mut Context) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();
        let size = libc::size_t::try_from(data.len()).map_err(io_err_other)?;
        self.wrap_write(cx, || {
            c_result!(unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, size) })
                .map(|res| res as usize)
        })
    }

    pub async fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        poll_fn(move |cx| self.poll_write(data, cx)).await
    }
}
