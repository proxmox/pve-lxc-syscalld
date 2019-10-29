use std::io;
use std::os::unix::io::RawFd;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

use crate::epoll::Epoll;

pub struct AssertSync<T>(pub T);
unsafe impl<T> Sync for AssertSync<T> {}

pub struct Reactor {
    epoll: Arc<Epoll>,
    removals: AssertSync<mpsc::Sender<RawFd>>,
    thread: JoinHandle<()>,
}

impl Reactor {
    pub fn new() -> io::Result<Self> {
        let epoll = Arc::new(Epoll::new()?);

        let (send_remove, recv_remove) = mpsc::channel();

        let handle = std::thread::spawn({
            let epoll = Arc::clone(&epoll);
            move || Self::thread_main(epoll, recv_remove)
        });

        Ok(Self {
            epoll,
            removals: AssertSync(send_remove),
            thread: handle,
        })
    }

    fn thread_main(epoll: Arc<Epoll>, removals: mpsc::Receiver<RawFd>) {
        let _ = epoll;
        let _ = removals;
    }
}
