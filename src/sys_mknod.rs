use failure::Error;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::syscall::SyscallStatus;

pub async fn mknod(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    let path = msg.arg_c_string(0)?;
    println!("Responding with ENOENT: {:?}", path);
    Ok(SyscallStatus::Err(libc::ENOENT))
}

pub async fn mknodat(_msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
    println!("Responding with ENOENT");
    Ok(SyscallStatus::Err(libc::ENOENT))
}
