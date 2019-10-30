use std::convert::TryFrom;
use std::io;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::task::{Context, Poll};

use crate::error::io_err_other;
use crate::poll_fn::poll_fn;
use crate::reactor::PolledFd;
use crate::rw_traits;
use crate::tools::Fd;

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

pub fn pipe() -> io::Result<(Pipe<rw_traits::Read>, Pipe<rw_traits::Write>)> {
    let mut pfd: [RawFd; 2] = [0, 0];

    c_try!(unsafe { libc::pipe2(pfd.as_mut_ptr(), libc::O_CLOEXEC) });

    let (fd_in, fd_out) = unsafe { (Fd::from_raw_fd(pfd[0]), Fd::from_raw_fd(pfd[1])) };
    let fd_in = PolledFd::new(fd_in)?;
    let fd_out = PolledFd::new(fd_out)?;

    Ok((
        Pipe {
            fd: fd_in,
            _phantom: PhantomData,
        },
        Pipe {
            fd: fd_out,
            _phantom: PhantomData,
        },
    ))
}

impl<RW: rw_traits::HasRead> Pipe<RW> {
    pub fn poll_read(&mut self, cx: &mut Context, data: &mut [u8]) -> Poll<io::Result<usize>> {
        let size = libc::size_t::try_from(data.len()).map_err(io_err_other)?;
        let fd = self.fd.as_raw_fd();
        self.fd.wrap_read(cx, || {
            c_result!(unsafe { libc::read(fd, data.as_mut_ptr() as *mut libc::c_void, size) })
                .map(|res| res as usize)
        })
    }

    pub async fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        poll_fn(move |cx| self.poll_read(cx, data)).await
    }
}

impl<RW: rw_traits::HasWrite> Pipe<RW> {
    pub fn poll_write(&mut self, data: &[u8], cx: &mut Context) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();
        let size = libc::size_t::try_from(data.len()).map_err(io_err_other)?;
        self.fd.wrap_write(cx, || {
            c_result!(unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, size) })
                .map(|res| res as usize)
        })
    }

    pub async fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        poll_fn(move |cx| self.poll_write(data, cx)).await
    }
}
