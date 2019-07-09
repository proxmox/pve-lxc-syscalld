use std::ffi::CString;
use std::os::unix::io::{AsRawFd, FromRawFd};

use failure::Error;
use nix::errno::Errno;
use nix::sys::stat;

use crate::fork::forking_syscall;
use crate::lxcseccomp::ProxyMessageBuffer;
use crate::pidfd::PidFd;
use crate::syscall::SyscallStatus;
use crate::tools::Fd;

pub async fn mknod(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let pathname = msg.arg_c_string(0)?;
    let mode = msg.arg_mode_t(1)?;
    let dev = msg.arg_dev_t(2)?;
    let cwd = msg.pid_fd().fd_cwd()?;

    let pidfd = unsafe { PidFd::from_raw_fd(msg.pid_fd().as_raw_fd()) };
    do_mknodat(pidfd, cwd, pathname, mode, dev).await
}

pub async fn mknodat(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let dirfd = msg.arg_fd(0, libc::O_DIRECTORY)?;
    let pathname = msg.arg_c_string(1)?;
    let mode = msg.arg_mode_t(2)?;
    let dev = msg.arg_dev_t(3)?;

    let pidfd = unsafe { PidFd::from_raw_fd(msg.pid_fd().as_raw_fd()) };
    do_mknodat(pidfd, dirfd, pathname, mode, dev).await
}

async fn do_mknodat(
    pidfd: PidFd,
    dirfd: Fd,
    _pathname: CString,
    _mode: stat::mode_t,
    _dev: stat::dev_t,
) -> Result<SyscallStatus, Error> {
    println!("=> Responding with ENOENT");
    Ok(forking_syscall(move || {
        pidfd.mount_namespace()?.setns()?;
        std::mem::drop(pidfd);
        std::mem::drop(dirfd);
        Ok(SyscallStatus::Err(libc::ENOENT))
    })
    .await?)
}
