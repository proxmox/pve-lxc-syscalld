use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::task::{Context, Poll};

use tokio::io::unix::AsyncFd;

use crate::tools::Fd;

#[repr(transparent)]
pub struct EventedFd {
    fd: Fd,
}

impl EventedFd {
    #[inline]
    pub fn new(fd: Fd) -> Self {
        Self { fd }
    }
}

impl AsRawFd for EventedFd {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl FromRawFd for EventedFd {
    #[inline]
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(unsafe { Fd::from_raw_fd(fd) })
    }
}

impl IntoRawFd for EventedFd {
    #[inline]
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_raw_fd()
    }
}

#[repr(transparent)]
pub struct PolledFd {
    fd: AsyncFd<EventedFd>,
}

impl PolledFd {
    pub fn new(fd: Fd) -> tokio::io::Result<Self> {
        Ok(Self {
            fd: AsyncFd::new(EventedFd::new(fd))?,
        })
    }

    pub fn wrap_read<T>(
        &self,
        cx: &mut Context,
        func: impl FnOnce() -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        let mut ready_guard = ready!(self.fd.poll_read_ready(cx))?;
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                ready_guard.clear_ready();
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }

    pub fn wrap_write<T>(
        &self,
        cx: &mut Context,
        func: impl FnOnce() -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        let mut ready_guard = ready!(self.fd.poll_write_ready(cx))?;
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                ready_guard.clear_ready();
                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl AsRawFd for PolledFd {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.get_ref().as_raw_fd()
    }
}

impl IntoRawFd for PolledFd {
    #[inline]
    fn into_raw_fd(self) -> RawFd {
        // for the kind of resource we're managing it should always be possible to extract it from
        // its driver
        self.fd.into_inner().into_raw_fd()
    }
}
