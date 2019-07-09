//! Fork helper.
//!
//! Note that forking in rust can be dangerous. A fork must consider all mutexes to be in a broken
//! state, and cannot rely on any of its reference life times, so we be careful what kind of data
//! we continue to work with.

use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::panic::UnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::poll_fn;
use futures::io::AsyncRead;

use crate::syscall::SyscallStatus;
use crate::tools::Fd;
use crate::{libc_try, libc_wrap};

pub async fn forking_syscall<F>(func: F) -> io::Result<SyscallStatus>
where
    F: FnOnce() -> io::Result<SyscallStatus> + UnwindSafe,
{
    let mut fork = Fork::new(func)?;
    let mut buf = [0u8; 10];

    use futures::io::AsyncReadExt;
    fork.read_exact(&mut buf).await?;
    fork.wait()?;

    Ok(SyscallStatus::Err(libc::ENOENT))
}

pub struct Fork {
    pid: Option<libc::pid_t>,
    // FIXME: abuse! tokio-fs is not updated to futures@0.3 yet, but a TcpStream does the same
    // thing as a file when it's already open anyway...
    out: crate::tools::GenericStream,
}

impl Drop for Fork {
    fn drop(&mut self) {
        if self.pid.is_some() {
            let _ = self.wait();
        }
    }
}

impl Fork {
    pub fn new<F>(func: F) -> io::Result<Self>
    where
        F: FnOnce() -> io::Result<SyscallStatus> + UnwindSafe,
    {
        let mut pipe: [c_int; 2] = [0, 0];
        libc_try!(unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) });
        let (pipe_r, pipe_w) = (Fd(pipe[0]), Fd(pipe[1]));

        let pipe_r = match crate::tools::GenericStream::from_fd(pipe_r) {
            Ok(o) => o,
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err.to_string())),
        };

        let pid = libc_try!(unsafe { libc::fork() });
        if pid == 0 {
            std::mem::drop(pipe_r);
            let mut pipe_w = unsafe { std::fs::File::from_raw_fd(pipe_w.into_raw_fd()) };

            let _ = std::panic::catch_unwind(move || {
                let mut buf = [0u8; 10];

                match func() {
                    Ok(SyscallStatus::Ok(value)) => unsafe {
                        std::ptr::write(buf.as_mut_ptr().add(1) as *mut i64, value);
                    },
                    Ok(SyscallStatus::Err(value)) => {
                        buf[0] = 1;
                        unsafe {
                            std::ptr::write(buf.as_mut_ptr().add(1) as *mut i32, value);
                        }
                    }
                    Err(err) => match err.raw_os_error() {
                        Some(err) => {
                            buf[0] = 2;
                            unsafe {
                                std::ptr::write(buf.as_mut_ptr().add(1) as *mut i32, err);
                            }
                        }
                        None => {
                            buf[0] = 3;
                        }
                    },
                }

                use std::io::Write;
                match pipe_w.write_all(&buf) {
                    Ok(()) => unsafe { libc::_exit(0) },
                    Err(_) => unsafe { libc::_exit(1) },
                }
            });
            unsafe {
                libc::_exit(-1);
            }
        }

        Ok(Self {
            pid: Some(pid),
            out: pipe_r,
        })
    }

    pub fn wait(&mut self) -> io::Result<()> {
        let my_pid = self.pid.take().unwrap();

        loop {
            let mut status: c_int = -1;
            match libc_wrap!(unsafe { libc::waitpid(my_pid, &mut status, 0) }) {
                Ok(pid) if pid == my_pid => break,
                Ok(_other) => continue,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(other) => return Err(other),
            }
        }

        Ok(())
    }

    pub async fn async_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        poll_fn(|cx| Pin::new(&mut *self).poll_read(cx, buf)).await
    }
}

// default impl will work
impl AsyncRead for Fork {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.out) }.poll_read(cx, buf)
    }

    unsafe fn initializer(&self) -> futures::io::Initializer {
        self.out.initializer()
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context,
        bufs: &mut [futures::io::IoSliceMut],
    ) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.out) }.poll_read_vectored(cx, bufs)
    }
}
