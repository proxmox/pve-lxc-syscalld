use std::io::{self, IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::ptr;
use std::task::{Context, Poll};

use anyhow::Error;
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, SockaddrLike};

use crate::io::polled_fd::PolledFd;
use crate::poll_fn::poll_fn;
use crate::tools::AssertSendSync;
use crate::tools::Fd;

fn seq_packet_socket(flags: SockFlag) -> nix::Result<Fd> {
    let fd = socket::socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        flags | SockFlag::SOCK_CLOEXEC,
        None,
    )?;
    Ok(unsafe { Fd::from_raw_fd(fd) })
}

pub struct SeqPacketListener {
    fd: PolledFd,
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
        socket::listen(fd.as_raw_fd(), 16)?;

        let fd = PolledFd::new(fd)?;

        Ok(Self { fd })
    }

    pub fn poll_accept(&mut self, cx: &mut Context) -> Poll<io::Result<SeqPacketSocket>> {
        let fd = self.as_raw_fd();
        let res = self.fd.wrap_read(cx, || {
            c_result!(unsafe {
                libc::accept4(fd, ptr::null_mut(), ptr::null_mut(), libc::SOCK_CLOEXEC)
            })
            .map(|fd| unsafe { Fd::from_raw_fd(fd as RawFd) })
        });
        match res {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(fd)) => Poll::Ready(SeqPacketSocket::new(fd)),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
        }
    }

    pub async fn accept(&mut self) -> io::Result<SeqPacketSocket> {
        poll_fn(move |cx| self.poll_accept(cx)).await
    }
}

pub struct SeqPacketSocket {
    fd: PolledFd,
}

impl AsRawFd for SeqPacketSocket {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl SeqPacketSocket {
    pub fn new(fd: Fd) -> io::Result<Self> {
        Ok(Self {
            fd: PolledFd::new(fd)?,
        })
    }

    pub fn poll_sendmsg(
        &self,
        cx: &mut Context,
        msg: &AssertSendSync<libc::msghdr>,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();

        self.fd.wrap_write(cx, || {
            c_result!(unsafe { libc::sendmsg(fd, &msg.0 as *const libc::msghdr, 0) })
                .map(|rc| rc as usize)
        })
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

        poll_fn(move |cx| self.poll_sendmsg(cx, &msg)).await
    }

    pub fn poll_recvmsg(
        &self,
        cx: &mut Context,
        msg: &mut AssertSendSync<libc::msghdr>,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();

        self.fd.wrap_read(cx, || {
            c_result!(unsafe { libc::recvmsg(fd, &mut msg.0 as *mut libc::msghdr, 0) })
                .map(|rc| rc as usize)
        })
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
            msg_iov: iov.as_ptr() as _,
            msg_iovlen: iov.len(),
            msg_control: cmsg_buf.as_mut_ptr() as *mut std::ffi::c_void,
            msg_controllen: cmsg_buf.len(),
            msg_flags: libc::MSG_CMSG_CLOEXEC,
        });

        let data_size = poll_fn(|cx| self.poll_recvmsg(cx, &mut msg)).await?;
        Ok((data_size, msg.0.msg_controllen as usize))
    }

    #[inline]
    pub fn shutdown(&self, how: socket::Shutdown) -> nix::Result<()> {
        socket::shutdown(self.as_raw_fd(), how)
    }
}
