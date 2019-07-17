use std::convert::TryFrom;
use std::ffi::CString;
use std::{io, mem, ptr};
use std::os::raw::{c_int, c_uint};

use failure::Error;
use nix::errno::Errno;

use crate::fork::forking_syscall;
use crate::lxcseccomp::ProxyMessageBuffer;
use crate::pidfd::{IdMap, PidFd};
use crate::sc_libc_try;
use crate::syscall::SyscallStatus;

/*
 * int quotactl(int cmd, const char *special, int id, caddr_t addr);
 *
 * `special` is a path to the mount point, so we need to be in the process' file system view
 * `addr` will point to a datastructure, so we should read a reasonable amount (a 4k page?) of
 *  memory from there, but only if the sub command type makes use of it.
 *
 *  Actually the libc crate contains most of the structures anyway!
 *
 * Cmd:
 *  QCMD(SubCmd, Type)
 *      Type: USRQUOTA | GRPQUOTA | PRJQUOTA (but we don't need to care)
 * SubCmd:
 *  |Done?         name           addr meaning|
 *    X       Q_QUOTAON     path to quota file
 *    X      Q_QUOTAOFF                ignored
 *    X      Q_GETQUOTA        struct dqblk {}
 *    X      Q_SETQUOTA        struct dqblk {}
 *    X       Q_GETINFO       struct dqinfo {}
 *    X        Q_GETFMT                [u8; 4]
 *    X       Q_SETQLIM                              -EOPNOTSUPP: not documented anymore
 *    X        Q_SETUSE                              -EOPNOTSUPP: not documented anymore
 *    X      Q_GETSTATS      struct dqstats {}       -EOPNOTSUPP: obsolete, removed since 2.4.22!
 *    X  Q_GETNEXTQUOTA    struct nextdqblk {}
 *    X       Q_SETINFO       struct dqinfo {}
 *    X          Q_SYNC                ignored       -EOPNOTSUPP if `special` is NULL!
 *
 * xfs stuff:
 *           Q_XQUOTAON           unsigned int
 *          Q_XQUOTAOFF           unsigned int
 *          ...
 *          (we don't actually have xfs containers atm...)
 */

const Q_GETNEXTQUOTA: c_int = 0x800009;

const KINDMASK: c_int = 0xff;
const SUBCMDSHIFT: c_int = 8;

#[repr(C)]
struct nextdqblk {
    dqblk: libc::dqblk,
    dqb_id: u32,
}

pub async fn quotactl(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let cmd = msg.arg_int(0)?;
    let special = msg.arg_opt_c_string(1)?;
    // let _id = msg.arg_int(2)?;
    // let _addr = msg.arg_caddr_t(3)?;

    // FIXME: We can *generally* check that `special` if not None points to a block device owned
    // by the container. On the other hand, the container should not have access to the device
    // anyway unless the `devices` cgroup allows it, and should not have been allowed to `mknod` a
    // device on a non-NODEV mounted file system.

    let kind = cmd & KINDMASK;
    let subcmd = ((cmd as c_uint) >> SUBCMDSHIFT) as c_int;
    match subcmd {
        libc::Q_GETINFO => q_getinfo(msg, cmd, special).await,
        libc::Q_SETINFO => q_setinfo(msg, cmd, special).await,
        libc::Q_GETFMT => q_getfmt(msg, cmd, special).await,
        libc::Q_QUOTAON => q_quotaon(msg, cmd, special).await,
        libc::Q_QUOTAOFF => q_quotaoff(msg, cmd, special).await,
        libc::Q_GETQUOTA => q_getquota(msg, cmd, special, kind).await,
        libc::Q_SETQUOTA => q_setquota(msg, cmd, special, kind).await,
        libc::Q_SYNC => q_sync(msg, cmd, special).await,
        Q_GETNEXTQUOTA => q_getnextquota(msg, cmd, special, kind).await,
        _ => {
            //eprintln!("Unhandled quota subcommand: {:x}", subcmd);
            Ok(Errno::EOPNOTSUPP.into())
        }
    }
}

//#[allow(non_camel_case_names)]
#[repr(C)]
struct dqinfo {
    dqi_bgrace: u64,
    dqi_igrace: u64,
    dqi_flags: u32,
    dqi_valid: u32,
}

pub async fn q_getinfo(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let id = msg.arg_int(2)?;
    let addr = msg.arg_caddr_t(3)? as u64;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let mut data: dqinfo = unsafe { mem::zeroed() };
        let special = special.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special, id, &mut data as *mut dqinfo as *mut i8)
        });

        msg.mem_write_struct(addr, &data)?;
        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_setinfo(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let special = match special {
        Some(s) => s,
        None => return Ok(Errno::EINVAL.into()),
    };
    let id = msg.arg_int(2)?;
    let mut data: dqinfo = msg.arg_struct_by_ptr(3)?;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special.as_ptr(), id, &mut data as *mut dqinfo as *mut i8)
        });

        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_getfmt(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let id = msg.arg_int(2)?;
    let addr = msg.arg_caddr_t(3)? as u64;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let mut data: u32 = 0;
        let special = special.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special, id, &mut data as *mut u32 as *mut i8)
        });

        msg.mem_write_struct(addr, &data)?;
        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_quotaon(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let id = msg.arg_int(2)?;
    let addr = msg.arg_c_string(3)?;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let special = special.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        let out = sc_libc_try!(unsafe { libc::quotactl(cmd, special, id, addr.as_ptr() as _) });

        Ok(SyscallStatus::Ok(out.into()))
    })
    .await?)
}

pub async fn q_quotaoff(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let id = msg.arg_int(2)?;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let special = special.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        let out = sc_libc_try!(unsafe { libc::quotactl(cmd, special, id, ptr::null_mut()) });

        Ok(SyscallStatus::Ok(out.into()))
    })
    .await?)
}

fn uid_gid_arg(
    msg: &ProxyMessageBuffer,
    arg: u32,
    kind: c_int,
) -> Result<(c_int, Option<IdMap>), Error> {
    let id = msg.arg_int(arg)?;
    let map = match kind {
        libc::USRQUOTA => msg.pid_fd().get_uid_map()?,
        libc::GRPQUOTA => msg.pid_fd().get_gid_map()?,
        _ => return Ok((id, None)),
    };

    let id = map
        .map_from(id as u64)
        .ok_or_else(|| Error::from(Errno::ERANGE))?;
    let id = c_int::try_from(id)
        .map_err(|_| Error::from(Errno::ERANGE))?;

    Ok((id, Some(map)))
}

pub async fn q_getquota(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
    kind: c_int,
) -> Result<SyscallStatus, Error> {
    let special = match special {
        Some(s) => s,
        None => return Ok(Errno::EINVAL.into()),
    };

    let (id, _) = uid_gid_arg(msg, 2, kind)?;
    let addr = msg.arg_caddr_t(3)? as u64;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let mut data: libc::dqblk = unsafe { mem::zeroed() };
        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special.as_ptr(), id, &mut data as *mut libc::dqblk as *mut i8)
        });

        msg.mem_write_struct(addr, &data)?;
        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_setquota(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
    kind: c_int,
) -> Result<SyscallStatus, Error> {
    let special = match special {
        Some(s) => s,
        None => return Ok(Errno::EINVAL.into()),
    };

    let (id, _) = uid_gid_arg(msg, 2, kind)?;
    let mut data: libc::dqblk = msg.arg_struct_by_ptr(3)?;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special.as_ptr(), id, &mut data as *mut libc::dqblk as *mut i8)
        });

        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_getnextquota(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
    kind: c_int,
) -> Result<SyscallStatus, Error> {
    let special = match special {
        Some(s) => s,
        None => return Ok(Errno::EINVAL.into()),
    };

    let (id, idmap) = uid_gid_arg(msg, 2, kind)?;
    let addr = msg.arg_caddr_t(3)? as u64;

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        let mut data: nextdqblk = unsafe { mem::zeroed() };
        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special.as_ptr(), id, &mut data as *mut nextdqblk as *mut i8)
        });

        if let Some(idmap) = idmap {
            data.dqb_id = idmap
                .map_into(u64::from(data.dqb_id))
                .ok_or_else(|| io::Error::from_raw_os_error(libc::ERANGE))? as u32;
        }

        msg.mem_write_struct(addr, &data)?;
        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}

pub async fn q_sync(
    msg: &ProxyMessageBuffer,
    cmd: c_int,
    special: Option<CString>,
) -> Result<SyscallStatus, Error> {
    let special = match special {
        Some(s) => s,
        None => return Ok(Errno::EINVAL.into()),
    };

    let caps = msg.pid_fd().user_caps()?;
    Ok(forking_syscall(move || {
        caps.apply(&PidFd::current()?)?;

        sc_libc_try!(unsafe {
            libc::quotactl(cmd, special.as_ptr(), 0, ptr::null_mut())
        });

        Ok(SyscallStatus::Ok(0))
    })
    .await?)
}
