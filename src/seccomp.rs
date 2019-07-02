//! Low level seccomp module
//!
//! Mostly provides data structures.

use std::io;
use std::os::raw::c_int;

/// Contains syscall data.
#[repr(C)]
pub struct SeccompData {
    pub nr: c_int,
    pub arch: u32,
    pub instruction_pointer: u64,
    pub args: [u64; 6],
}

/// Seccomp syscall notification data.
///
/// Sent by the kernel when a seccomp filter returns `SECCOMP_RET_USER_NOTIF` for a syscall.
#[repr(C)]
pub struct SeccompNotif {
    pub id: u64,
    pub pid: u32,
    pub flags: u32,
    pub data: SeccompData,
}

/// Seccomp syscall response data.
///
/// This is sent as a reply to `SeccompNotif`.
#[repr(C)]
pub struct SeccompNotifResp {
    pub id: u64,
    pub val: i64,
    pub error: i32,
    pub flags: u32,
}

/// Information about the actual sizes of `SeccompNotif`, and `SeccompNotifResp` and `SeccompData`.
///
/// If the sizes mismatch it is likely that the kernel has an incompatible view of these data
/// structures.
#[derive(Clone)]
#[repr(C)]
pub struct SeccompNotifSizes {
    pub notif: u16,
    pub notif_resp: u16,
    pub data: u16,
}

impl SeccompNotifSizes {
    /// Query the kernel for its data structure sizes.
    pub fn get() -> io::Result<Self> {
        const SECCOMP_GET_NOTIF_SIZES: c_int = 3;

        let mut this = Self {
            notif: 0,
            notif_resp: 0,
            data: 0,
        };

        let rc = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                SECCOMP_GET_NOTIF_SIZES,
                0,
                &mut this as *mut _,
            )
        };
        if rc == 0 {
            Ok(this)
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
