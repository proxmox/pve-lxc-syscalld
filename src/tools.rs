//! Various utilities.
//!
//! Note that this should stay small, otherwise we should introduce a dependency on our `proxmox`
//! crate as that's where we have all this stuff usually...

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

pub fn set_fd_nonblocking<T: AsRawFd + ?Sized>(fd: &T, on: bool) -> nix::Result<libc::c_int> {
    use nix::fcntl;
    let fd = fd.as_raw_fd();
    let mut flags = fcntl::OFlag::from_bits(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFL)?).unwrap();
    flags.set(fcntl::OFlag::O_NONBLOCK, on);
    fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFL(flags))
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
        unsafe {
            let data = std::alloc::alloc(std::alloc::Layout::array::<u8>(len).unwrap());
            Vec::from_raw_parts(data, len, len)
        }
    }
}

pub trait FromFd {
    fn from_fd<T: IntoRawFd>(fd: T) -> Self;
}

impl<T: FromRawFd> FromFd for T {
    fn from_fd<F: IntoRawFd>(fd: F) -> Self {
        unsafe { Self::from_raw_fd(fd.into_raw_fd()) }
    }
}

/// This is totally unsafe. Only use this when you know what you're doing.
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct AssertSendSync<T>(pub T);
unsafe impl<T> Send for AssertSendSync<T> {}
unsafe impl<T> Sync for AssertSendSync<T> {}
