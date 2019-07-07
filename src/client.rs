use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::sync::Arc;

use failure::{format_err, Error};

use crate::lxcseccomp::ProxyMessageBuffer;
use crate::socket::AsyncSeqPacketSocket;
use crate::{SyscallMeta, SyscallStatus};

pub struct Client {
    socket: AsyncSeqPacketSocket,
}

impl Client {
    pub fn new(socket: AsyncSeqPacketSocket) -> Arc<Self> {
        Arc::new(Self { socket })
    }

    /// Wrapp futures returning a `Result` so if they fail we `shutdown()` the socket to drop the
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

            let (size, mut fds) = {
                let mut iovec = msg.io_vec_mut();
                self.socket.recv_fds_vectored(&mut iovec, 1).await?
            };

            if size == 0 {
                eprintln!("client disconnected");
                break Ok(());
            }

            msg.set_len(size)?;

            let mut fds = fds.drain(..);
            let memory = fds
                .next()
                .ok_or_else(|| format_err!("did not receive memory file descriptor from liblxc"))?;

            std::mem::drop(fds);

            let meta = SyscallMeta {
                memory: unsafe { std::fs::File::from_raw_fd(memory.into_raw_fd()) },
            };

            // Note: our spawned tasks here must not access our socket, as we cannot guarantee
            // they'll be woken up if another task errors into `wrap_error()`.
            tokio::spawn(
                self.clone()
                    .wrap_error(self.clone().__handle_syscall(msg, meta)),
            );
        }
    }

    // Note: we must not use the socket for anything other than sending the result!
    async fn __handle_syscall(
        self: Arc<Self>,
        mut msg: ProxyMessageBuffer,
        meta: SyscallMeta,
    ) -> Result<(), Error> {
        let result = Self::handle_syscall(&msg, meta).await?;

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

        let iovec = msg.io_vec_no_cookie();
        self.socket.sendmsg_vectored(&iovec).await?;

        Ok(())
    }

    async fn handle_syscall(
        msg: &ProxyMessageBuffer,
        meta: SyscallMeta,
    ) -> Result<SyscallStatus, Error> {
        match msg.request().data.nr as i64 {
            libc::SYS_mknod => crate::sys_mknod::mknod(msg, meta).await,
            libc::SYS_mknodat => crate::sys_mknod::mknodat(msg, meta).await,
            _ => Ok(SyscallStatus::Err(libc::ENOSYS)),
        }
    }
}
