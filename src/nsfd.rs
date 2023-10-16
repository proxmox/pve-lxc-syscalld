//! Namespace file descriptor helpers.

use std::ffi::CStr;
use std::io;
use std::marker::PhantomData;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

pub mod ns_type {
    pub trait NsType {
        const TYPE: libc::c_int;
    }

    macro_rules! define_ns_type {
        ($name:ident, $number:expr) => {
            pub struct $name;
            impl NsType for $name {
                const TYPE: libc::c_int = $number;
            }
        };
    }

    define_ns_type!(Mount, libc::CLONE_NEWNS);
    define_ns_type!(User, libc::CLONE_NEWUSER);
    define_ns_type!(Cgroup, libc::CLONE_NEWCGROUP);
}

pub use ns_type::NsType;

file_descriptor_type!(RawNsFd);

impl RawNsFd {
    pub fn open(path: &CStr) -> io::Result<Self> {
        Self::openat(libc::AT_FDCWD, path)
    }

    pub fn openat(fd: RawFd, path: &CStr) -> io::Result<Self> {
        let fd =
            c_try!(unsafe { libc::openat(fd, path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) });

        Ok(unsafe { Self::from_raw_fd(fd) })
    }

    pub fn setns(&self, ns_type: c_int) -> io::Result<()> {
        c_try!(unsafe { libc::setns(self.as_raw_fd(), ns_type) });
        Ok(())
    }
}

#[repr(transparent)]
pub struct NsFd<T: NsType>(RawNsFd, PhantomData<T>);

impl<T: NsType> std::ops::Deref for NsFd<T> {
    type Target = RawNsFd;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: NsType> NsFd<T> {
    pub fn open(path: &CStr) -> io::Result<Self> {
        Ok(Self(RawNsFd::open(path)?, PhantomData))
    }

    pub fn openat(fd: RawFd, path: &CStr) -> io::Result<Self> {
        Ok(Self(RawNsFd::openat(fd, path)?, PhantomData))
    }

    pub fn setns(&self) -> io::Result<()> {
        self.0.setns(T::TYPE)
    }
}
