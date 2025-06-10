use std::io::{self, IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr;

use anyhow::Error;
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, SockaddrLike};
use tokio::io::unix::AsyncFd;

use crate::tools::AssertSendSync;

fn seq_packet_socket(flags: SockFlag) -> nix::Result<OwnedFd> {
    socket::socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        flags | SockFlag::SOCK_CLOEXEC,
        None,
    )
}

pub struct SeqPacketListener {
    fd: AsyncFd<OwnedFd>,
}

impl AsRawFd for SeqPacketListener {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl SeqPacketListener {
    pub fn bind(address: &dyn SockaddrLike) -> Result<Self, Error> {
        let fd = seq_packet_socket(SockFlag::empty())?;
        socket::bind(fd.as_raw_fd(), address)?;
        socket::listen(
            &fd,
            socket::Backlog::new(16).expect("backlog of 16 should be valid"),
        )?;

        let fd = AsyncFd::new(fd)?;

        Ok(Self { fd })
    }

    pub async fn accept(&mut self) -> io::Result<SeqPacketSocket> {
        let fd = super::wrap_read(&self.fd, |fd| {
            c_result!(unsafe {
                libc::accept4(
                    fd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                )
            })
        })
        .await?;

        let fd = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
        SeqPacketSocket::new(fd)
    }
}

pub struct SeqPacketSocket {
    fd: AsyncFd<OwnedFd>,
}

impl AsRawFd for SeqPacketSocket {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl SeqPacketSocket {
    pub fn new(fd: OwnedFd) -> io::Result<Self> {
        Ok(Self {
            fd: AsyncFd::new(fd)?,
        })
    }

    async fn sendmsg(&self, msg: &AssertSendSync<libc::msghdr>) -> io::Result<usize> {
        let rc = super::wrap_write(&self.fd, |fd| {
            c_result!(unsafe { libc::sendmsg(fd, &msg.0 as *const libc::msghdr, 0) })
        })
        .await?;
        Ok(rc as usize)
    }

    pub async fn sendmsg_vectored(&self, iov: &[IoSlice<'_>]) -> io::Result<usize> {
        let msg = AssertSendSync(libc::msghdr {
            msg_name: ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: iov.as_ptr() as _,
            msg_iovlen: iov.len(),
            msg_control: ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        });

        self.sendmsg(&msg).await
    }

    async fn recvmsg(&self, msg: &mut AssertSendSync<libc::msghdr>) -> io::Result<usize> {
        let rc = super::wrap_read(&self.fd, move |fd| {
            c_result!(unsafe { libc::recvmsg(fd, &mut msg.0 as *mut libc::msghdr, 0) })
        })
        .await?;
        Ok(rc as usize)
    }

    // clippy is wrong about this one
    #[allow(clippy::needless_lifetimes)]
    pub async fn recvmsg_vectored(
        &self,
        iov: &mut [IoSliceMut<'_>],
        cmsg_buf: &mut [u8],
    ) -> io::Result<(usize, usize)> {
        let mut msg = AssertSendSync(libc::msghdr {
            msg_name: ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: iov.as_mut_ptr() as _,
            msg_iovlen: iov.len(),
            msg_control: cmsg_buf.as_mut_ptr() as *mut std::ffi::c_void,
            msg_controllen: cmsg_buf.len(),
            msg_flags: libc::MSG_CMSG_CLOEXEC,
        });

        let data_size = self.recvmsg(&mut msg).await?;
        Ok((data_size, msg.0.msg_controllen))
    }

    #[inline]
    pub fn shutdown(&self, how: socket::Shutdown) -> nix::Result<()> {
        socket::shutdown(self.as_raw_fd(), how)
    }
}
