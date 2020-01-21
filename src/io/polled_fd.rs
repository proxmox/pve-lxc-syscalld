use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::task::{Context, Poll};

use mio::event::Evented;
use mio::unix::EventedFd as MioEventedFd;
use mio::Poll as MioPoll;
use mio::{PollOpt, Ready, Token};
use tokio::io::PollEvented;

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
        Self::new(Fd::from_raw_fd(fd))
    }
}

impl IntoRawFd for EventedFd {
    #[inline]
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_raw_fd()
    }
}

impl Evented for EventedFd {
    fn register(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        MioEventedFd(self.fd.as_ref()).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        MioEventedFd(self.fd.as_ref()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &MioPoll) -> io::Result<()> {
        MioEventedFd(self.fd.as_ref()).deregister(poll)
    }
}

#[repr(transparent)]
pub struct PolledFd {
    fd: PollEvented<EventedFd>,
}

impl PolledFd {
    pub fn new(fd: Fd) -> tokio::io::Result<Self> {
        Ok(Self {
            fd: PollEvented::new(EventedFd::new(fd))?,
        })
    }

    pub fn wrap_read<T>(
        &self,
        cx: &mut Context,
        func: impl FnOnce() -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        ready!(self.fd.poll_read_ready(cx, mio::Ready::readable()))?;
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                self.fd.clear_read_ready(cx, mio::Ready::readable())?;
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
        ready!(self.fd.poll_write_ready(cx))?;
        match func() {
            Ok(out) => Poll::Ready(Ok(out)),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                self.fd.clear_write_ready(cx)?;
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
        self.fd
            .into_inner()
            .expect("failed to remove polled file descriptor from reactor")
            .into_raw_fd()
    }
}
