//! Various utilities.
//!
//! Note that this should stay small, otherwise we should introduce a dependency on our `proxmox`
//! crate as that's where we have all this stuff usually...

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub use io_uring::iovec::{IoVec, IoVecMut};

/// Guard a raw file descriptor with a drop handler. This is mostly useful when access to an owned
/// `RawFd` is required without the corresponding handler object (such as when only the file
/// descriptor number is required in a closure which may be dropped instead of being executed).
#[repr(transparent)]
pub struct Fd(pub RawFd);

file_descriptor_impl!(Fd);

impl FromRawFd for Fd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(fd)
    }
}

impl Fd {
    pub fn set_nonblocking(&self, nb: bool) -> std::io::Result<()> {
        let fd = self.as_raw_fd();
        let flags = c_try!(unsafe { libc::fcntl(fd, libc::F_GETFL) });
        let flags = if nb {
            flags | libc::O_NONBLOCK
        } else {
            flags & !libc::O_NONBLOCK
        };
        c_try!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) });
        Ok(())
    }
}

/// Byte vector utilities.
pub mod vec {
    /// Create an uninitialized byte vector of a specific size.
    ///
    /// This is just a shortcut for:
    /// ```no_run
    /// # let len = 64usize;
    /// let mut v = Vec::<u8>::with_capacity(len);
    /// unsafe {
    ///     v.set_len(len);
    /// }
    /// ```
    ///
    /// # Safety
    ///
    /// This is generally safe to call, but the contents of the vector are undefined.
    #[inline]
    pub unsafe fn uninitialized(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        out.set_len(len);
        out
    }
}

pub trait FromFd {
    fn from_fd(fd: Fd) -> Self;
}

impl<T: FromRawFd> FromFd for T {
    fn from_fd(fd: Fd) -> Self {
        unsafe { Self::from_raw_fd(fd.into_raw_fd()) }
    }
}
