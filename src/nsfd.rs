use std::io;
use std::marker::PhantomData;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::path::Path;

use crate::tools::path_ptr;
use crate::{file_descriptor_type, libc_try};

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
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::openat(libc::AT_FDCWD, path.as_ref())
    }

    pub fn openat<P: AsRef<Path>>(fd: RawFd, path: P) -> io::Result<Self> {
        let fd = libc_try!(unsafe {
            libc::openat(
                fd,
                path_ptr(path.as_ref()),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        });

        Ok(Self(fd))
    }

    pub fn setns(&self, ns_type: c_int) -> io::Result<()> {
        libc_try!(unsafe { libc::setns(self.0, ns_type) });
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
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Ok(Self(RawNsFd::open(path.as_ref())?, PhantomData))
    }

    pub fn openat<P: AsRef<Path>>(fd: RawFd, path: P) -> io::Result<Self> {
        Ok(Self(RawNsFd::openat(fd, path.as_ref())?, PhantomData))
    }

    pub fn setns(&self) -> io::Result<()> {
        self.0.setns(T::TYPE)
    }
}
