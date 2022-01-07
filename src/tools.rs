//! Various utilities.
//!
//! Note that this should stay small, otherwise we should introduce a dependency on our `proxmox`
//! crate as that's where we have all this stuff usually...

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

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
    pub fn set_nonblocking(&mut self, nb: bool) -> nix::Result<libc::c_int> {
        use nix::fcntl;
        let mut flags =
            fcntl::OFlag::from_bits(fcntl::fcntl(self.0, fcntl::FcntlArg::F_GETFL)?).unwrap();
        flags.set(fcntl::OFlag::O_NONBLOCK, nb);
        fcntl::fcntl(self.0, fcntl::FcntlArg::F_SETFL(flags))
    }
}

impl AsRef<RawFd> for Fd {
    #[inline]
    fn as_ref(&self) -> &RawFd {
        &self.0
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
        let data = std::alloc::alloc(std::alloc::Layout::array::<u8>(len).unwrap());
        Vec::from_raw_parts(data as *mut u8, len, len)
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

/// This is totally unsafe. Only use this when you know what you're doing.
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct AssertSendSync<T>(pub T);
unsafe impl<T> Send for AssertSendSync<T> {}
unsafe impl<T> Sync for AssertSendSync<T> {}
