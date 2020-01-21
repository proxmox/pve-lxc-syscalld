use std::convert::TryFrom;
use std::io;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::io_err_other;
use crate::io::polled_fd::PolledFd;
use crate::io::rw_traits;
use crate::tools::Fd;

pub use rw_traits::{Read, Write};

pub struct Pipe<RW> {
    fd: PolledFd,
    _phantom: PhantomData<RW>,
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
        self.fd.into_raw_fd()
    }
}

pub fn pipe() -> io::Result<(Pipe<rw_traits::Read>, Pipe<rw_traits::Write>)> {
    let mut pfd: [RawFd; 2] = [0, 0];

    c_try!(unsafe { libc::pipe2(pfd.as_mut_ptr(), libc::O_CLOEXEC) });

    let (fd_in, fd_out) = unsafe { (Fd::from_raw_fd(pfd[0]), Fd::from_raw_fd(pfd[1])) };

    Ok((
        Pipe {
            fd: PolledFd::new(fd_in)?,
            _phantom: PhantomData,
        },
        Pipe {
            fd: PolledFd::new(fd_out)?,
            _phantom: PhantomData,
        },
    ))
}

impl<RW: rw_traits::HasRead> AsyncRead for Pipe<RW> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.fd.wrap_read(cx, || {
            let fd = self.as_raw_fd();
            let size = libc::size_t::try_from(buf.len()).map_err(io_err_other)?;
            c_result!(unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, size) })
                .map(|res| res as usize)
        })
    }
}

impl<RW: rw_traits::HasWrite> AsyncWrite for Pipe<RW> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.fd.wrap_write(cx, || {
            let fd = self.as_raw_fd();
            let size = libc::size_t::try_from(buf.len()).map_err(io_err_other)?;
            c_result!(unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, size) })
                .map(|res| res as usize)
        })
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
