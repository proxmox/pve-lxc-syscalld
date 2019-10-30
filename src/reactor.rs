use std::convert::TryFrom;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;

use crate::epoll::{Epoll, EpollEvent, EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLOUT};
use crate::error::io_err_other;
use crate::poll_fn::poll_fn;
use crate::tools::Fd;

pub struct AssertSync<T>(pub T);
unsafe impl<T> Sync for AssertSync<T> {}

pub const READY_IN: u32 = 0b001;
pub const READY_OUT: u32 = 0b010;
pub const READY_ERR: u32 = 0b100;

pub struct Reactor {
    epoll: Arc<Epoll>,
    removed: Mutex<Vec<(RawFd, Box<RegistrationInner>)>>,
    thread: JoinHandle<()>,
}

impl Reactor {
    pub fn new() -> io::Result<Arc<Self>> {
        let epoll = Arc::new(Epoll::new()?);

        let handle = std::thread::spawn({
            let epoll = Arc::clone(&epoll);
            move || Self::thread_main(epoll)
        });

        Ok(Arc::new(Self {
            epoll,
            removed: Mutex::new(Vec::new()),
            thread: handle,
        }))
    }

    fn thread_main(epoll: Arc<Epoll>) {
        let mut buf: [EpollEvent; 16] = unsafe { std::mem::zeroed() };
        loop {
            let count = match epoll.wait(&mut buf, None) {
                Ok(count) => count,
                Err(err) => {
                    eprintln!("error in epoll loop: {}", err);
                    std::process::exit(1);
                }
            };
            for i in 0..count {
                Self::handle_event(&buf[i]);
            }
        }
    }

    fn handle_event(event: &EpollEvent) {
        let registration = unsafe { &mut *(event.r#u64 as *mut RegistrationInner) };
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
            fd,
            //ready: AtomicU32::new(0),
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

        self.epoll.add_fd(fd, EPOLLIN | EPOLLOUT, inner_ptr)?;

        Ok(Registration { inner: Some(inner) })
    }

    fn deregister(&self, registration: Box<RegistrationInner>) {
        self.removed
            .lock()
            .unwrap()
            .push((registration.fd, registration));
    }
}

pub struct Registration {
    // pin the data in memory because the other thread will access it
    // ManuallyDrop::take is nightly only :<
    inner: Option<Box<RegistrationInner>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let reactor = Arc::clone(&self.inner.as_ref().unwrap().reactor);
        reactor.deregister(self.inner.take().unwrap());
    }
}

// This is accessed by the reactor
struct RegistrationInner {
    fd: RawFd,
    //ready: AtomicU32,
    reactor: Arc<Reactor>,
    read_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
}

pub struct PolledFd {
    fd: Fd,
    registration: Registration,
}

impl PolledFd {
    pub fn new(fd: Fd, reactor: Arc<Reactor>) -> io::Result<Self> {
        let registration = reactor.register(fd.as_raw_fd())?;
        Ok(Self { fd, registration })
    }
}

impl PolledFd {
    pub fn poll_read(&mut self, data: &mut [u8], cx: &mut Context) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();
        let size = libc::size_t::try_from(data.len()).map_err(io_err_other)?;
        let mut read_waker = self
            .registration
            .inner
            .as_ref()
            .unwrap()
            .read_waker
            .lock()
            .unwrap();
        match c_result!(unsafe { libc::read(fd, data.as_mut_ptr() as *mut libc::c_void, size) }) {
            Ok(got) => Poll::Ready(Ok(got as usize)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                *read_waker = Some(cx.waker().clone());
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }

    pub async fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        poll_fn(move |cx| self.poll_read(data, cx)).await
    }
}
