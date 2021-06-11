use std::ffi::CString;
use std::os::unix::io::AsRawFd;

use anyhow::Error;
use nix::errno::Errno;
use nix::sys::stat;

use crate::fork::forking_syscall;
use crate::lxcseccomp::ProxyMessageBuffer;
use crate::process::PidFd;
use crate::sc_libc_try;
use crate::syscall::SyscallStatus;
use crate::tools::Fd;

pub async fn mknod(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let mode = msg.arg_mode_t(1)?;
    let dev = msg.arg_dev_t(2)?;
    if !check_mknod_dev(mode, dev) {
        return Ok(Errno::EPERM.into());
    }

    let pathname = msg.arg_c_string(0)?;
    let cwd = msg.pid_fd().fd_cwd()?;

    do_mknodat(msg.pid_fd(), cwd, pathname, mode, dev).await
}

pub async fn mknodat(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let mode = msg.arg_mode_t(2)?;
    let dev = msg.arg_dev_t(3)?;
    if !check_mknod_dev(mode, dev) {
        return Ok(Errno::EPERM.into());
    }

    let dirfd = msg.arg_fd(0, libc::O_DIRECTORY)?;
    let pathname = msg.arg_c_string(1)?;

    do_mknodat(msg.pid_fd(), dirfd, pathname, mode, dev).await
}

fn check_mknod_dev(mode: stat::mode_t, dev: stat::dev_t) -> bool {
    let sflag = mode & libc::S_IFMT;
    let major = stat::major(dev);
    let minor = stat::minor(dev);

    #[allow(clippy::match_like_matches_macro)]
    match (sflag, major, minor) {
        (libc::S_IFREG, 0, 0) => true, // touch
        (libc::S_IFCHR, 0, 0) => true, // whiteout
        (libc::S_IFCHR, 5, 0) => true, // /dev/tty
        (libc::S_IFCHR, 5, 1) => true, // /dev/console
        (libc::S_IFCHR, 5, 2) => true, // /dev/ptmx
        (libc::S_IFCHR, 1, 3) => true, // /dev/null
        (libc::S_IFCHR, 1, 5) => true, // /dev/zero
        (libc::S_IFCHR, 1, 7) => true, // /dev/full
        (libc::S_IFCHR, 1, 8) => true, // /dev/random
        (libc::S_IFCHR, 1, 9) => true, // /dev/urandom
        _ => false,
    }
}

async fn do_mknodat(
    pidfd: &PidFd,
    dirfd: Fd,
    pathname: CString,
    mode: stat::mode_t,
    dev: stat::dev_t,
) -> Result<SyscallStatus, Error> {
    let caps = pidfd.user_caps()?;

    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;
        let out =
            sc_libc_try!(unsafe { libc::mknodat(dirfd.as_raw_fd(), pathname.as_ptr(), mode, dev) });
        Ok(SyscallStatus::Ok(out.into()))
    })
    .await?)
}
