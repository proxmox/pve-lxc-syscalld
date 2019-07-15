//! Various utilities.
//!
//! Note that this should stay small, otherwise we should introduce a dependency on our `proxmox`
//! crate as that's where we have all this stuff usually...

use std::io;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::io::{AsyncRead, AsyncWrite};
use futures::ready;
use mio::unix::EventedFd;
use mio::{PollOpt, Ready, Token};

#[macro_export]
macro_rules! file_descriptor_type {
    ($type:ident) => {
        #[repr(transparent)]
        pub struct $type(RawFd);

        crate::file_descriptor_impl!($type);

        impl FromRawFd for $type {
            unsafe fn from_raw_fd(fd: RawFd) -> Self {
                Self(fd)
            }
        }
    };
}

#[macro_export]
macro_rules! file_descriptor_impl {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                if self.0 >= 0 {
                    unsafe {
                        libc::close(self.0);
                    }
                }
            }
        }

        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                self.0
            }
        }

        impl IntoRawFd for $type {
            fn into_raw_fd(mut self) -> RawFd {
                let fd = self.0;
                self.0 = -libc::EBADF;
                fd
            }
        }
    };
}

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

pub struct AsyncFd {
    fd: Fd,
    registration: tokio::reactor::Registration,
}

impl Drop for AsyncFd {
    fn drop(&mut self) {
        if let Err(err) = self.registration.deregister(&self.fd) {
            eprintln!("failed to deregister I/O resource with reactor: {}", err);
        }
    }
}

impl AsyncFd {
    pub fn new(fd: Fd) -> io::Result<Self> {
        let registration = tokio::reactor::Registration::new();
        if !registration.register(&fd)? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "duplicate file descriptor registration?",
            ));
        }

        Ok(Self { fd, registration })
    }

    pub fn poll_read_ready(&self, cx: &mut Context) -> Poll<io::Result<mio::Ready>> {
        self.registration.poll_read_ready(cx)
    }

    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<io::Result<mio::Ready>> {
        self.registration.poll_write_ready(cx)
    }
}

impl AsRawFd for AsyncFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

// At the time of writing, tokio-fs in master was disabled as it wasn't updated to futures@0.3 yet.
pub struct GenericStream(Option<AsyncFd>);

impl GenericStream {
    pub fn from_fd(fd: Fd) -> io::Result<Self> {
        AsyncFd::new(fd).map(|fd| Self(Some(fd)))
    }

    fn raw_fd(&self) -> RawFd {
        self.0
            .as_ref()
            .map(|fd| fd.as_raw_fd())
            .unwrap_or(-libc::EBADF)
    }

    pub fn poll_read_ready(&self, cx: &mut Context) -> Poll<io::Result<mio::Ready>> {
        match self.0 {
            Some(ref fd) => fd.poll_read_ready(cx),
            None => Poll::Ready(Err(io::ErrorKind::InvalidInput.into())),
        }
    }

    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<io::Result<mio::Ready>> {
        match self.0 {
            Some(ref fd) => fd.poll_write_ready(cx),
            None => Poll::Ready(Err(io::ErrorKind::InvalidInput.into())),
        }
    }
}

impl AsyncRead for GenericStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let res = unsafe { libc::read(self.raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
            if res >= 0 {
                return Poll::Ready(Ok(res as usize));
            }

            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                match ready!(self.poll_read_ready(cx)) {
                    Ok(_) => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
            return Poll::Ready(Err(err));
        }
    }
}

impl AsyncWrite for GenericStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            let res = unsafe { libc::write(self.raw_fd(), buf.as_ptr() as *const _, buf.len()) };
            if res >= 0 {
                return Poll::Ready(Ok(res as usize));
            }

            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                match ready!(self.poll_write_ready(cx)) {
                    Ok(_) => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
            return Poll::Ready(Err(err));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<io::Result<()>> {
        std::mem::drop(self.get_mut().0.take());
        Poll::Ready(Ok(()))
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

#[macro_export]
macro_rules! c_call {
    ($expr:expr) => {{
        let res = $expr;
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok::<_, ::std::io::Error>(res)
        }
    }};
}

#[macro_export]
macro_rules! c_try {
    ($expr:expr) => {
        crate::c_call!($expr)?
    };
}

pub trait FromFd {
    fn from_fd(fd: Fd) -> Self;
}

impl<T: FromRawFd> FromFd for T {
    fn from_fd(fd: Fd) -> Self {
        unsafe { Self::from_raw_fd(fd.into_raw_fd()) }
    }
}
