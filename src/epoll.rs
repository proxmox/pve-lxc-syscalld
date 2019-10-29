use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use crate::tools::Fd;

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

    fn add_fd(&self, fd: RawFd, events: u32, data: u64) -> io::Result<()> {
        self.addmod_fd(libc::EPOLL_CTL_ADD, fd, events, data)
    }

    fn modify_fd(&self, fd: RawFd, events: u32, data: u64) -> io::Result<()> {
        self.addmod_fd(libc::EPOLL_CTL_MOD, fd, events, data)
    }

    fn remove_fd(&self, fd: RawFd) -> io::Result<()> {
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
}
