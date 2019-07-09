use std::ffi::CString;

use failure::Error;
use nix::errno::Errno;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::tools::vec;

pub enum SyscallStatus {
    Ok(i64),
    Err(i32),
}

pub fn get_c_string(msg: &ProxyMessageBuffer, offset: u64) -> Result<CString, Error> {
    let mut data = unsafe { vec::uninitialized(4096) };
    let got = msg.mem_fd().read_at(&mut data, offset)?;

    let len = unsafe { libc::strnlen(data.as_ptr() as *const _, got) };
    if len >= got {
        Err(nix::Error::Sys(Errno::EINVAL).into())
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
            return Ok($crate::syscall::SyscallStatus::Err(::nix::errno::errno() as _))
        } else {
            res
        }
    }};
}
