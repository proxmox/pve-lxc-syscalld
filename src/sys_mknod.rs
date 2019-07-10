use std::ffi::CString;
use std::os::unix::io::AsRawFd;

use failure::Error;
use nix::sys::stat;

use crate::fork::forking_syscall;
use crate::lxcseccomp::ProxyMessageBuffer;
use crate::pidfd::PidFd;
use crate::syscall::SyscallStatus;
use crate::tools::Fd;
use crate::{libc_try, sc_libc_try};

pub async fn mknod(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let mode = msg.arg_mode_t(1)?;
    let dev = msg.arg_dev_t(2)?;
    if !check_mknod_dev(mode, dev) {
        return Ok(SyscallStatus::Err(libc::EPERM));
    }

    let pathname = msg.arg_c_string(0)?;
    let cwd = msg.pid_fd().fd_cwd()?;

    do_mknodat(msg.pid_fd(), cwd, pathname, mode, dev).await
}

pub async fn mknodat(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let mode = msg.arg_mode_t(2)?;
    let dev = msg.arg_dev_t(3)?;
    if !check_mknod_dev(mode, dev) {
        return Ok(SyscallStatus::Err(libc::EPERM));
    }

    let dirfd = msg.arg_fd(0, libc::O_DIRECTORY)?;
    let pathname = msg.arg_c_string(1)?;

    do_mknodat(msg.pid_fd(), dirfd, pathname, mode, dev).await
}

fn check_mknod_dev(mode: stat::mode_t, dev: stat::dev_t) -> bool {
    let sflag = mode & libc::S_IFMT;
    let major = stat::major(dev);
    let minor = stat::minor(dev);

    match (sflag, major, minor) {
        (libc::S_IFCHR, 1, 3) => true,
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

    // FIXME: !!! ALSO COPY THE PROCESS' CAPABILITY SET AND USE KEEP_CAPS!

    Ok(forking_syscall(move || {
        caps.apply_cgroups()?;
        pidfd.mount_namespace()?.setns()?;
        pidfd.chroot()?;
        libc_try!(unsafe { libc::fchdir(dirfd.as_raw_fd()) });
        caps.apply_user_caps()?;
        let out =
            sc_libc_try!(unsafe { libc::mknodat(dirfd.as_raw_fd(), pathname.as_ptr(), mode, dev) });
        Ok(SyscallStatus::Ok(out.into()))
    })
    .await?)
}
