use std::sync::Arc;

use failure::Error;

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::socket::AsyncSeqPacketSocket;
use crate::SyscallStatus;

pub struct Client {
    socket: AsyncSeqPacketSocket,
}

impl Client {
    pub fn new(socket: AsyncSeqPacketSocket) -> Arc<Self> {
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
                eprintln!("client disconnected");
                break Ok(());
            }

            // Note: our spawned tasks here must not access our socket, as we cannot guarantee
            // they'll be woken up if another task errors into `wrap_error()`.
            tokio::spawn(
                self.clone()
                    .wrap_error(self.clone().__handle_syscall(msg)),
            );
        }
    }

    // Note: we must not use the socket for anything other than sending the result!
    async fn __handle_syscall(
        self: Arc<Self>,
        mut msg: ProxyMessageBuffer,
    ) -> Result<(), Error> {
        let result = Self::handle_syscall(&msg).await?;

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

    async fn handle_syscall(
        msg: &ProxyMessageBuffer,
    ) -> Result<SyscallStatus, Error> {
        match msg.request().data.nr as i64 {
            libc::SYS_mknod => crate::sys_mknod::mknod(msg).await,
            libc::SYS_mknodat => crate::sys_mknod::mknodat(msg).await,
            _ => Ok(SyscallStatus::Err(libc::ENOSYS)),
        }
    }
}
