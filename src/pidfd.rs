//! pidfd helper functionality

use std::ffi::{CStr, CString};
use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use crate::nsfd::{ns_type, NsFd};
use crate::tools::Fd;
use crate::{file_descriptor_type, libc_try};

file_descriptor_type!(PidFd);

impl PidFd {
    pub fn open(pid: libc::pid_t) -> io::Result<Self> {
        let path = CString::new(format!("/proc/{}", pid)).unwrap();

        let fd =
            libc_try!(unsafe { libc::open(path.as_ptr(), libc::O_DIRECTORY | libc::O_CLOEXEC) });

        Ok(Self(fd))
    }

    pub fn mount_namespace(&self) -> io::Result<NsFd<ns_type::Mount>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/mnt\0")
        })
    }

    pub fn cgroup_namespace(&self) -> io::Result<NsFd<ns_type::Cgroup>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/cgroup\0")
        })
    }

    pub fn user_namespace(&self) -> io::Result<NsFd<ns_type::User>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/user\0")
        })
    }

    fn fd(&self, path: &[u8], flags: c_int) -> io::Result<Fd> {
        Ok(Fd(libc_try!(unsafe {
            libc::openat(
                self.as_raw_fd(),
                path.as_ptr() as *const _,
                flags | libc::O_CLOEXEC,
            )
        })))
    }

    pub fn fd_cwd(&self) -> io::Result<Fd> {
        self.fd(b"cwd\0", libc::O_DIRECTORY)
    }

    pub fn fd_num(&self, num: RawFd, flags: c_int) -> io::Result<Fd> {
        self.fd(format!("fd/{}\0", num).as_bytes(), flags)
    }

    //pub fn dup(&self) -> io::Result<Self> {
    //    Ok(Self(libc_try!(unsafe {
    //        libc::fcntl(self.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0)
    //    })))
    //}
}
