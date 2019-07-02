use std::convert::TryFrom;
use std::task::Context;
use std::task::Poll;
use std::os::raw::c_void;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{io, mem, ptr};

use failure::{bail, Error};
use futures::ready;
use futures::future::poll_fn;
use nix::sys::socket::{AddressFamily, SockAddr, SockFlag, SockType};

use super::tools::{vec, Fd};

pub struct SeqPacketSocket(Fd);

impl FromRawFd for SeqPacketSocket {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(Fd(fd))
    }
}

impl SeqPacketSocket {
    fn fd(&self) -> RawFd {
        (self.0).0
    }

    pub fn recv_fds(&mut self, data: &mut [u8], num_fds: usize) -> io::Result<(usize, Vec<Fd>)> {
        let fdlist_size = u32::try_from(mem::size_of::<RawFd>() * num_fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("size error: {}", e)))?;

        let mut cmsgbuf = unsafe { vec::uninitialized(libc::CMSG_SPACE(fdlist_size) as usize) };
        unsafe {
            ptr::write_bytes(cmsgbuf.as_mut_ptr(), 0xff, cmsgbuf.len());
        }

        let mut iov = [libc::iovec {
            iov_base: data.as_mut_ptr() as *mut c_void,
            iov_len: data.len(),
        }];

        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = iov.as_mut_ptr() as *mut libc::iovec;
        msg.msg_iovlen = iov.len();
        msg.msg_controllen = cmsgbuf.len();
        msg.msg_control = cmsgbuf.as_mut_ptr() as *mut c_void;
        let _ = &cmsgbuf; // from now on we only use raw pointer stuff

        let received = unsafe {
            libc::recvmsg(self.fd(), &mut msg, libc::MSG_CMSG_CLOEXEC)
        };
        if received < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut out_fds = Vec::with_capacity(num_fds);
        let mut cmsg_ptr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
        while !cmsg_ptr.is_null() {
            let cmsg: &libc::cmsghdr = unsafe { &*cmsg_ptr };
            if cmsg.cmsg_type == libc::SCM_RIGHTS
                && cmsg.cmsg_len == unsafe { libc::CMSG_LEN(fdlist_size) as usize }
                && cmsg.cmsg_level == libc::SOL_SOCKET
            {
                let fds = unsafe {
                    std::slice::from_raw_parts(libc::CMSG_DATA(cmsg_ptr) as *const RawFd, num_fds)
                };
                for fd in fds {
                    println!("Received fd: {}", *fd);
                    out_fds.push(Fd(*fd));
                }
                break;
            }
            cmsg_ptr = unsafe { libc::CMSG_NXTHDR(&msg, cmsg_ptr) };
        }

        Ok((received as usize, out_fds))
    }

    /// Send a message via `sendmsg(2)`.
    ///
    /// Note that short writes are silently treated as success, since this is a `SOCK_SEQPACKET`,
    /// so neither continuing nor repeating a partial messages makes all that much sense.
    pub fn sendmsg(&mut self, data: &[u8]) -> io::Result<()> {
        let mut iov = [libc::iovec {
            iov_base: data.as_ptr() as *const c_void as *mut c_void,
            iov_len: data.len(),
        }];

        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = iov.as_mut_ptr() as *mut libc::iovec;
        msg.msg_iovlen = iov.len();

        let sent = unsafe { libc::sendmsg(self.fd(), &mut msg, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }

        // XXX: what to do with short writes? we're a SEQPACKET socket...

        Ok(())
    }

    fn as_fd(&self) -> &Fd {
        &self.0
    }
}

impl AsRawFd for SeqPacketSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd()
    }
}

pub struct SeqPacketListener {
    fd: Fd,
    registration: tokio::reactor::Registration,
}

impl Drop for SeqPacketListener {
    fn drop(&mut self) {
        if let Err(err) = self.registration.deregister(&self.fd) {
            eprintln!("failed to deregister I/O resource with reactor: {}", err);
        }
    }
}

impl SeqPacketListener {
    pub fn bind(address: &SockAddr) -> Result<Self, Error> {
        let fd = Fd(nix::sys::socket::socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )?);

        nix::sys::socket::bind(fd.as_raw_fd(), &address)?;
        nix::sys::socket::listen(fd.as_raw_fd(), 16)?;

        let registration = tokio::reactor::Registration::new();
        if !registration.register(&fd)? {
            bail!("duplicate file descriptor registration?");
        }

        Ok(Self {
            fd,
            registration,
        })
    }

    pub fn poll_accept(
        &mut self,
        cx: &mut Context,
    ) -> Poll<io::Result<AsyncSeqPacketSocket>> {
        let fd = loop {
            match nix::sys::socket::accept4(
                self.fd.as_raw_fd(),
                SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            ) {
                Ok(fd) => break Fd(fd),
                Err(err) => match err.as_errno() {
                    Some(nix::errno::Errno::EAGAIN) => {
                        match ready!(self.registration.poll_read_ready(cx)) {
                            Ok(_) => continue,
                            Err(err) => return Poll::Ready(Err(err)),
                        }
                    }
                    Some(other) => {
                        return Poll::Ready(Err(io::Error::from_raw_os_error(other as _)));
                    }
                    None => {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::Other,
                            "unexpected non-OS error in nix::sys::socket::accept4()",
                        )));
                    },
                }
            };
        };

        Poll::Ready(match AsyncSeqPacketSocket::new(fd) {
            Ok(c) => Ok(c),
            Err(err) => Err(io::Error::new(io::ErrorKind::Other, err.to_string())),
        })
    }

    pub async fn accept(&mut self) -> io::Result<AsyncSeqPacketSocket> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }
}

// Do I care about having it as a stream?
//#[must_use = "streams do nothing unless polled"]
//pub struct SeqPacketIncoming {
//}

pub struct AsyncSeqPacketSocket {
    socket: SeqPacketSocket,
    registration: tokio::reactor::Registration,
}

impl Drop for AsyncSeqPacketSocket {
    fn drop(&mut self) {
        if let Err(err) = self.registration.deregister(self.socket.as_fd()) {
            eprintln!("failed to deregister I/O resource with reactor: {}", err);
        }
    }
}

impl AsyncSeqPacketSocket {
    pub fn new(fd: Fd) -> Result<Self, Error> {
        let registration = tokio::reactor::Registration::new();
        if !registration.register(&fd)? {
            bail!("duplicate file descriptor registration?");
        }

        Ok(Self {
            socket: unsafe { SeqPacketSocket::from_raw_fd(fd.into_raw_fd()) },
            registration,
        })
    }

    pub fn poll_recv_fds(
        &mut self,
        data: &mut [u8],
        num_fds: usize,
        cx: &mut Context,
    ) -> Poll<io::Result<(usize, Vec<Fd>)>> {
        loop {
            match self.socket.recv_fds(data, num_fds) {
                Ok(res) => break Poll::Ready(Ok(res)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    match ready!(self.registration.poll_read_ready(cx)) {
                        Ok(_) => continue,
                        Err(err) => break Poll::Ready(Err(err)),
                    }
                },
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }

    pub async fn recv_fds(
        &mut self,
        data: &mut [u8],
        num_fds: usize,
    ) -> io::Result<(usize, Vec<Fd>)> {
        poll_fn(move |cx| self.poll_recv_fds(data, num_fds, cx)).await
    }

    pub fn poll_sendmsg(&mut self, data: &[u8], cx: &mut Context) -> Poll<io::Result<()>> {
        loop {
            match self.socket.sendmsg(data) {
                Ok(res) => break Poll::Ready(Ok(res)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    match ready!(self.registration.poll_write_ready(cx)) {
                        Ok(_) => continue,
                        Err(err) => break Poll::Ready(Err(err)),
                    }
                },
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }

    pub async fn sendmsg(&mut self, data: &[u8]) -> io::Result<()> {
        poll_fn(move |cx| self.poll_sendmsg(data, cx)).await
    }
}
