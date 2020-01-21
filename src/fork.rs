//! Fork helper.
//!
//! Note that forking in rust can be dangerous. A fork must consider all mutexes to be in a broken
//! state, and cannot rely on any of its reference life times, so we be careful what kind of data
//! we continue to work with.

use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::panic::UnwindSafe;

use tokio::io::AsyncReadExt;

use crate::io::pipe::{self, Pipe};
use crate::syscall::SyscallStatus;
use crate::tools::Fd;

pub async fn forking_syscall<F>(func: F) -> io::Result<SyscallStatus>
where
    F: FnOnce() -> io::Result<SyscallStatus> + UnwindSafe,
{
    let mut fork = Fork::new(func)?;
    let result = fork.get_result().await?;
    fork.wait()?;
    Ok(result)
}

pub struct Fork {
    pid: Option<libc::pid_t>,
    // FIXME: abuse! tokio-fs is not updated to futures@0.3 yet, but a TcpStream does the same
    // thing as a file when it's already open anyway...
    out: Pipe<pipe::Read>,
}

impl Drop for Fork {
    fn drop(&mut self) {
        if self.pid.is_some() {
            let _ = self.wait();
        }
    }
}

#[repr(C, packed)]
struct Data {
    val: i64,
    error: i32,
    failure: i32,
}

impl Fork {
    pub fn new<F>(func: F) -> io::Result<Self>
    where
        F: FnOnce() -> io::Result<SyscallStatus> + UnwindSafe,
    {
        let (pipe_r, pipe_w) = pipe::pipe()?;

        let pid = c_try!(unsafe { libc::fork() });
        if pid == 0 {
            drop(pipe_r);
            let mut pipe_w = unsafe { Fd::from_raw_fd(pipe_w.into_raw_fd()) };
            let _ = std::panic::catch_unwind(move || {
                pipe_w.set_nonblocking(false).unwrap();
                let mut pipe_w = unsafe { std::fs::File::from_raw_fd(pipe_w.into_raw_fd()) };
                let out = match func() {
                    Ok(SyscallStatus::Ok(val)) => Data {
                        val,
                        error: 0,
                        failure: 0,
                    },
                    Ok(SyscallStatus::Err(error)) => Data {
                        val: -1,
                        error: error as _,
                        failure: 0,
                    },
                    Err(err) => Data {
                        val: -1,
                        error: -1,
                        failure: err.raw_os_error().unwrap_or(libc::EFAULT),
                    },
                };

                let slice = unsafe {
                    std::slice::from_raw_parts(
                        &out as *const Data as *const u8,
                        std::mem::size_of::<Data>(),
                    )
                };

                use std::io::Write;
                match pipe_w.write_all(slice) {
                    Ok(()) => unsafe { libc::_exit(0) },
                    Err(_) => unsafe { libc::_exit(1) },
                }
            });
            unsafe {
                libc::_exit(-1);
            }
        }
        drop(pipe_w);

        Ok(Self {
            pid: Some(pid),
            out: pipe_r,
        })
    }

    pub fn wait(&mut self) -> io::Result<()> {
        let my_pid = self.pid.take().unwrap();
        let mut status: c_int = -1;

        loop {
            match c_result!(unsafe { libc::waitpid(my_pid, &mut status, 0) }) {
                Ok(pid) if pid == my_pid => break,
                Ok(_other) => continue,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(other) => return Err(other),
            }
        }

        if status != 0 {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "error in child process",
            ))
        } else {
            Ok(())
        }
    }

    pub async fn get_result(&mut self) -> io::Result<SyscallStatus> {
        let mut data: Data = unsafe { std::mem::zeroed() };
        // Compiler bug: we currently need to put the slice into a temporary variable...
        let dataslice: &mut [u8] = unsafe {
            std::slice::from_raw_parts_mut(
                &mut data as *mut Data as *mut u8,
                std::mem::size_of::<Data>(),
            )
        };
        self.out.read_exact(dataslice).await?;
        //self.read_exact(unsafe {
        //    std::slice::from_raw_parts_mut(
        //        &mut data as *mut Data as *mut u8,
        //        std::mem::size_of::<Data>(),
        //    )
        //})
        //.await?;
        if data.failure != 0 {
            Err(io::Error::from_raw_os_error(data.failure))
        } else if data.error == 0 {
            Ok(SyscallStatus::Ok(data.val))
        } else {
            Ok(SyscallStatus::Err(data.error))
        }
    }
}
