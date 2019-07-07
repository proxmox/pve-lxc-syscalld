use failure::Error;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::{SyscallMeta, SyscallStatus};

pub async fn mknod(_msg: &ProxyMessageBuffer, _meta: SyscallMeta) -> Result<SyscallStatus, Error> {
    println!("Responding with ENOENT");
    Ok(SyscallStatus::Err(libc::ENOENT))
}

pub async fn mknodat(_msg: &ProxyMessageBuffer, _meta: SyscallMeta) -> Result<SyscallStatus, Error> {
    println!("Responding with ENOENT");
    Ok(SyscallStatus::Err(libc::ENOENT))
}
