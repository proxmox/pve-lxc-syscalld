use failure::Error;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::SyscallStatus;

pub async fn mknod(_msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    println!("Responding with ENOENT");
    Ok(SyscallStatus::Err(libc::ENOENT))
}

pub async fn mknodat(_msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    println!("Responding with ENOENT");
    Ok(SyscallStatus::Err(libc::ENOENT))
}
