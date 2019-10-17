use std::sync::Arc;

use failure::Error;
use nix::errno::Errno;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::syscall::{self, Syscall, SyscallStatus};
use io_uring::socket::SeqPacketSocket;

pub struct Client {
    socket: SeqPacketSocket,
}

impl Client {
    pub fn new(socket: SeqPacketSocket) -> Arc<Self> {
        Arc::new(Self { socket })
    }

    /// Wrap futures returning a `Result` so if they fail we `shutdown()` the socket to drop the
    /// client.
    async fn wrap_error<F>(self: Arc<Self>, fut: F)
    where
        F: std::future::Future<Output = Result<(), Error>>,
    {
        if let Err(err) = fut.await {
            eprintln!("client error, dropping connection: {}", err);
            if let Err(err) = self.socket.shutdown(nix::sys::socket::Shutdown::Both) {
                eprintln!("    (error shutting down client socket: {})", err);
            }
        }
    }

    pub async fn main(self: Arc<Self>) {
        self.clone().wrap_error(self.main_do()).await
    }

    async fn main_do(self: Arc<Self>) -> Result<(), Error> {
        loop {
            let mut msg = ProxyMessageBuffer::new(64);

            if !msg.recv(&self.socket).await? {
                break Ok(());
            }

            // Note: our spawned tasks here must not access our socket, as we cannot guarantee
            // they'll be woken up if another task errors into `wrap_error()`.
            crate::spawn(self.clone().wrap_error(self.clone().__handle_syscall(msg)));
        }
    }

    // Note: we must not use the socket for anything other than sending the result!
    async fn __handle_syscall(self: Arc<Self>, mut msg: ProxyMessageBuffer) -> Result<(), Error> {
        let result = match Self::handle_syscall(&msg).await {
            Ok(r) => r,
            Err(err) => {
                if let Some(errno) = err.downcast_ref::<nix::errno::Errno>() {
                    SyscallStatus::Err(*errno as _)
                } else if let Some(nix::Error::Sys(errno)) = err.downcast_ref::<nix::Error>() {
                    SyscallStatus::Err(*errno as _)
                } else if let Some(ioerr) = err.downcast_ref::<std::io::Error>() {
                    if let Some(errno) = ioerr.raw_os_error() {
                        SyscallStatus::Err(errno)
                    } else {
                        return Err(err);
                    }
                } else {
                    return Err(err);
                }
            }
        };

        let resp = msg.response_mut();
        match result {
            SyscallStatus::Ok(val) => {
                resp.val = val;
                resp.error = 0;
            }
            SyscallStatus::Err(err) => {
                resp.val = -1;
                resp.error = -err;
            }
        }

        msg.respond(&self.socket).await.map_err(Error::from)
    }

    async fn handle_syscall(msg: &ProxyMessageBuffer) -> Result<SyscallStatus, Error> {
        let (arch, sysnr) = (msg.request().data.arch, msg.request().data.nr);

        let syscall_nr = match syscall::translate_syscall(arch, sysnr) {
            Some(nr) => nr,
            None => return Ok(Errno::ENOSYS.into()),
        };

        match syscall_nr {
            Syscall::Mknod => crate::sys_mknod::mknod(msg).await,
            Syscall::MknodAt => crate::sys_mknod::mknodat(msg).await,
            Syscall::Quotactl => crate::sys_quotactl::quotactl(msg).await,
        }
    }
}
