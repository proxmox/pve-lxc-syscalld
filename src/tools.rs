//! Various utilities.
//!
//! Note that this should stay small, otherwise we should introduce a dependency on our `proxmox`
//! crate as that's where we have all this stuff usually...

use std::io;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use mio::unix::EventedFd;
use mio::{PollOpt, Ready, Token};

/// Guard a raw file descriptor with a drop handler. This is mostly useful when access to an owned
/// `RawFd` is required without the corresponding handler object (such as when only the file
/// descriptor number is required in a closure which may be dropped instead of being executed).
#[repr(transparent)]
pub struct Fd(pub RawFd);

impl Drop for Fd {
    fn drop(&mut self) {
        if self.0 != -1 {
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl IntoRawFd for Fd {
    fn into_raw_fd(mut self) -> RawFd {
        let fd = self.0;
        self.0 = -1;
        fd
    }
}

impl FromRawFd for Fd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(fd)
    }
}

impl mio::Evented for Fd {
    fn register(
        &self,
        poll: &mio::Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        poll.register(&EventedFd(&self.0), token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &mio::Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        poll.reregister(&EventedFd(&self.0), token, interest, opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        poll.deregister(&EventedFd(&self.0))
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
    #[inline]
    pub unsafe fn uninitialized(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        out.set_len(len);
        out
    }
}

/// The standard IoSlice does not implement Send and Sync. These types do.
pub struct IoVec<'a> {
    _iov: libc::iovec,
    _phantom: PhantomData<&'a [u8]>,
}

unsafe impl Send for IoVec<'_> {}
unsafe impl Sync for IoVec<'_> {}

impl IoVec<'_> {
    pub fn new(slice: &[u8]) -> Self {
        Self {
            _iov: libc::iovec {
                iov_base: slice.as_ptr() as *mut libc::c_void,
                iov_len: slice.len(),
            },
            _phantom: PhantomData,
        }
    }
}

pub struct IoVecMut<'a> {
    _iov: libc::iovec,
    _phantom: PhantomData<&'a [u8]>,
}

unsafe impl Send for IoVecMut<'_> {}
unsafe impl Sync for IoVecMut<'_> {}

impl IoVecMut<'_> {
    pub fn new(slice: &mut [u8]) -> Self {
        Self {
            _iov: libc::iovec {
                iov_base: slice.as_mut_ptr() as *mut libc::c_void,
                iov_len: slice.len(),
            },
            _phantom: PhantomData,
        }
    }
}
