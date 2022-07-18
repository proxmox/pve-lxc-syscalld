use std::io;
use std::os::unix::io::{AsRawFd, RawFd};

use tokio::io::unix::AsyncFd;

use crate::tools::Fd;

pub mod cmsg;
pub mod pipe;
pub mod rw_traits;
pub mod seq_packet;

pub async fn wrap_read<R, F>(async_fd: &AsyncFd<Fd>, mut call: F) -> io::Result<R>
where
    F: FnMut(RawFd) -> io::Result<R>,
{
    let fd = async_fd.as_raw_fd();
    loop {
        let mut guard = async_fd.readable().await?;

        match call(fd) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                continue;
            }
            other => return other,
        }
    }
}

pub async fn wrap_write<R, F>(async_fd: &AsyncFd<Fd>, mut call: F) -> io::Result<R>
where
    F: FnMut(RawFd) -> io::Result<R>,
{
    let fd = async_fd.as_raw_fd();
    loop {
        let mut guard = async_fd.writable().await?;

        match call(fd) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                continue;
            }
            other => return other,
        }
    }
}
