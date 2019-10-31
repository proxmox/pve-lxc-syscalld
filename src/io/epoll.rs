use std::convert::TryFrom;
use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::time::Duration;

use crate::error::io_err_other;
use crate::tools::Fd;

pub type EpollEvent = libc::epoll_event;

pub const EPOLLIN: u32 = libc::EPOLLIN as u32;
pub const EPOLLET: u32 = libc::EPOLLET as u32;
pub const EPOLLOUT: u32 = libc::EPOLLOUT as u32;
pub const EPOLLERR: u32 = libc::EPOLLERR as u32;
pub const EPOLLHUP: u32 = libc::EPOLLHUP as u32;

pub struct Epoll {
    fd: Fd,
}

impl Epoll {
    pub fn new() -> io::Result<Self> {
        let fd = unsafe { Fd::from_raw_fd(c_try!(libc::epoll_create1(libc::EPOLL_CLOEXEC))) };
        Ok(Self { fd })
    }

    pub fn add_file<T: AsRawFd>(&self, fd: &T, events: u32, data: u64) -> io::Result<()> {
        self.add_fd(fd.as_raw_fd(), events, data)
    }

    pub fn modify_file<T: AsRawFd>(&self, fd: &T, events: u32, data: u64) -> io::Result<()> {
        self.modify_fd(fd.as_raw_fd(), events, data)
    }

    pub fn remove_file<T: AsRawFd>(&self, fd: &T) -> io::Result<()> {
        self.remove_fd(fd.as_raw_fd())
    }

    fn addmod_fd(&self, op: c_int, fd: RawFd, events: u32, data: u64) -> io::Result<()> {
        let mut events = libc::epoll_event {
            events,
            r#u64: data,
        };
        c_try!(unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), op, fd, &mut events) });
        Ok(())
    }

    pub fn add_fd(&self, fd: RawFd, events: u32, data: u64) -> io::Result<()> {
        self.addmod_fd(libc::EPOLL_CTL_ADD, fd, events, data)
    }

    pub fn modify_fd(&self, fd: RawFd, events: u32, data: u64) -> io::Result<()> {
        self.addmod_fd(libc::EPOLL_CTL_MOD, fd, events, data)
    }

    pub fn remove_fd(&self, fd: RawFd) -> io::Result<()> {
        c_try!(unsafe {
            libc::epoll_ctl(
                self.fd.as_raw_fd(),
                libc::EPOLL_CTL_DEL,
                fd,
                std::ptr::null_mut(),
            )
        });
        Ok(())
    }

    pub fn wait(
        &self,
        event_buf: &mut [EpollEvent],
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        let millis = timeout
            .map(|t| c_int::try_from(t.as_millis()))
            .transpose()
            .map_err(io_err_other)?
            .unwrap_or(-1);
        let epfd = self.fd.as_raw_fd();
        let buf_len = c_int::try_from(event_buf.len()).map_err(io_err_other)?;
        let rc = c_try!(unsafe { libc::epoll_wait(epfd, event_buf.as_mut_ptr(), buf_len, millis) });
        Ok(rc as usize)
    }
}
