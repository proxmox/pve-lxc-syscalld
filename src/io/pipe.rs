use std::convert::{TryFrom, TryInto};
use std::io;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::io::rw_traits;

pub use rw_traits::{Read, Write};

/// Helper struct for generating pipes.
///
/// `Pipe` is a tokio-io supported type, associated with a reactor. After a `fork()` we cannot do
/// anything with it, including turning it into a raw fd as tokio will attempt to disassociate it
/// from the reactor, which will just break.
///
/// So we start out with this type which can be "upgraded" or "downgraded" into a `Pipe<T>` or
/// `OwnedFd`.
pub struct PipeFd<RW>(OwnedFd, PhantomData<RW>);

impl<RW> PipeFd<RW> {
    pub fn new(fd: OwnedFd) -> Self {
        Self(fd, PhantomData)
    }

    pub fn into_fd(self) -> OwnedFd {
        self.0
    }
}

pub fn pipe_fds() -> io::Result<(PipeFd<rw_traits::Read>, PipeFd<rw_traits::Write>)> {
    let mut pfd: [RawFd; 2] = [0, 0];

    c_try!(unsafe { libc::pipe2(pfd.as_mut_ptr(), libc::O_CLOEXEC) });

    let (fd_in, fd_out) = unsafe { (OwnedFd::from_raw_fd(pfd[0]), OwnedFd::from_raw_fd(pfd[1])) };

    Ok((PipeFd::new(fd_in), PipeFd::new(fd_out)))
}

/// Tokio supported pipe file descriptor. `tokio::fs::File` requires tokio's complete file system
/// feature gate, so we just use this `AsyncFd` wrapper.
pub struct Pipe<RW> {
    fd: AsyncFd<OwnedFd>,
    _phantom: PhantomData<RW>,
}

impl<RW> TryFrom<PipeFd<RW>> for Pipe<RW> {
    type Error = io::Error;

    fn try_from(fd: PipeFd<RW>) -> io::Result<Self> {
        Ok(Self {
            fd: AsyncFd::new(fd.into_fd())?,
            _phantom: PhantomData,
        })
    }
}

impl<RW> AsRawFd for Pipe<RW> {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl<RW> IntoRawFd for Pipe<RW> {
    #[inline]
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_inner().into_raw_fd()
    }
}

pub fn pipe() -> io::Result<(Pipe<rw_traits::Read>, Pipe<rw_traits::Write>)> {
    let (fd_in, fd_out) = pipe_fds()?;

    Ok((fd_in.try_into()?, fd_out.try_into()?))
}

impl<RW: rw_traits::HasRead> AsyncRead for Pipe<RW> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        let mut guard = ready!(self.fd.poll_read_ready(cx))?;

        let fd = self.as_raw_fd();
        let mem = buf.initialize_unfilled();
        match c_result!(unsafe { libc::read(fd, mem.as_mut_ptr() as *mut libc::c_void, mem.len()) })
        {
            Ok(received) => {
                if received > 0 {
                    buf.advance(received as usize)
                }
                guard.retain_ready();
                Poll::Ready(Ok(()))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                Poll::Pending
            }
            Err(err) => {
                guard.retain_ready();
                Poll::Ready(Err(err))
            }
        }
    }
}

impl<RW: rw_traits::HasWrite> AsyncWrite for Pipe<RW> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut guard = ready!(self.fd.poll_write_ready(cx))?;

        let fd = self.as_raw_fd();
        match c_result!(unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) })
        {
            Ok(res) => {
                guard.retain_ready();
                Poll::Ready(Ok(res as usize))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                Poll::Pending
            }
            Err(err) => {
                guard.retain_ready();
                Poll::Ready(Err(err))
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
