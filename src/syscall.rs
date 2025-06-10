use std::ffi::CString;
use std::os::raw::c_int;

use anyhow::Error;
use nix::errno::Errno;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::tools::vec;

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const AUDIT_ARCH_I386: u32 = 0x4000_0003;

pub enum SyscallStatus {
    Ok(i64),
    Err(i32),
}

impl From<Errno> for SyscallStatus {
    fn from(errno: Errno) -> Self {
        SyscallStatus::Err(errno as i32)
    }
}

#[derive(Debug)]
pub enum Syscall {
    Mknod,
    MknodAt,
    Quotactl,
}

pub struct SyscallArch {
    arch: u32,
    mknod: i32,
    mknodat: i32,
    quotactl: i32,
}

const SYSCALL_TABLE: &[SyscallArch] = &[
    SyscallArch {
        arch: AUDIT_ARCH_X86_64,
        mknod: 133,
        mknodat: 259,
        quotactl: 179,
    },
    SyscallArch {
        arch: AUDIT_ARCH_I386,
        mknod: 14,
        mknodat: 297,
        quotactl: 131,
    },
];

pub fn translate_syscall(arch: u32, nr: c_int) -> Option<Syscall> {
    if nr == -1 {
        // so we don't hit a -1 in SYSCALL_TABLE by accident...
        return None;
    }

    for sc in SYSCALL_TABLE {
        if sc.arch == arch {
            if nr == sc.mknod {
                return Some(Syscall::Mknod);
            } else if nr == sc.mknodat {
                return Some(Syscall::MknodAt);
            } else if nr == sc.quotactl {
                return Some(Syscall::Quotactl);
            }
        }
    }
    None
}

pub fn get_c_string(msg: &ProxyMessageBuffer, offset: u64) -> Result<CString, Error> {
    let mut data = unsafe { vec::uninitialized(4096) };
    let got = msg.mem_fd().read_at(&mut data, offset)?;

    let len = unsafe { libc::strnlen(data.as_ptr() as *const _, got) };
    if len >= got {
        Err(Errno::EINVAL.into())
    } else {
        unsafe {
            data.set_len(len);
        }
        // We used strlen, so the only Error in CString::new() cannot happen at this point:
        Ok(CString::new(data).unwrap())
    }
}

#[macro_export]
macro_rules! sc_libc_try {
    ($expr:expr) => {{
        let res = $expr;
        if res == -1 {
            return Ok($crate::syscall::SyscallStatus::Err(Errno::last_raw()));
        } else {
            res
        }
    }};
}
