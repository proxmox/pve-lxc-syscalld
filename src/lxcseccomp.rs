//! Module for LXC specific seccomp handling.

use std::convert::TryFrom;
use std::ffi::CString;
use std::os::raw::{c_int, c_uint};
use std::os::unix::fs::FileExt;
use std::os::unix::io::{FromRawFd, RawFd};
use std::{io, mem};

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use libc::pid_t;
use nix::errno::Errno;

use crate::io::cmsg;
use crate::io::iovec::{IoVec, IoVecMut};
use crate::io::seq_packet::SeqPacketSocket;
use crate::process::PidFd;
use crate::seccomp::{SeccompNotif, SeccompNotifResp, SeccompNotifSizes};
use crate::tools::{Fd, FromFd};

/// Seccomp notification proxy message sent by the lxc monitor.
///
/// Whenever a process in a container triggers a seccomp notification, and lxc has a seccomp
/// notification proxy configured, this is sent over to the proxy, together with a `SeccompNotif`,
/// `SeccompNotifResp` and a cookie.
///
/// Using this struct may be inconvenient. See the [`ProxyMessageBuffer`] for a convenient helper
/// for communcation.
#[repr(C)]
pub struct SeccompNotifyProxyMsg {
    /// Reserved data must be zero.
    reserved0: u64,

    /// The lxc monitor pid.
    ///
    /// Unless some other proxy forwards proxy messages, this should be the same pid as the peer
    /// we receive this message from.
    monitor_pid: pid_t,

    /// The container's init pid.
    ///
    /// If supported by the kernel, the lxc monitor should keep a pidfd open to this process, so
    /// this pid should be valid as long as `monitor_pid` is valid.
    init_pid: pid_t,

    /// Information about the seccomp structure sizes.
    ///
    /// This must be equal to `SeccompNotifSizes::get()`, otherwise the proxy and lxc monitor have
    /// inconsistent views of the kernel's seccomp API.
    sizes: SeccompNotifSizes,

    /// The length of the container's configured `lxc.seccomp.notify.cookie` value.
    cookie_len: u64,
}

/// Helper to receive and verify proxy notification messages.
pub struct ProxyMessageBuffer {
    proxy_msg: SeccompNotifyProxyMsg,
    seccomp_notif: SeccompNotif,
    seccomp_resp: SeccompNotifResp,
    cookie_buf: Vec<u8>,

    sizes: SeccompNotifSizes,
    seccomp_packet_size: usize,

    pid_fd: Option<PidFd>,
    mem_fd: Option<std::fs::File>,
}

unsafe fn io_vec_mut<T>(value: &mut T) -> IoVecMut {
    IoVecMut::new(std::slice::from_raw_parts_mut(
        value as *mut T as *mut u8,
        mem::size_of::<T>(),
    ))
}

unsafe fn io_vec<T>(value: &T) -> IoVec {
    IoVec::new(std::slice::from_raw_parts(
        value as *const T as *const u8,
        mem::size_of::<T>(),
    ))
}

lazy_static! {
    static ref SECCOMP_SIZES: SeccompNotifSizes = SeccompNotifSizes::get_checked()
        .map_err(|e| panic!("{}\nrefusing to run", e))
        .unwrap();
}

impl ProxyMessageBuffer {
    /// Allocate a new proxy message buffer with a specific maximum cookie size.
    pub fn new(max_cookie: usize) -> Self {
        let sizes = SECCOMP_SIZES.clone();

        let seccomp_packet_size = mem::size_of::<SeccompNotifyProxyMsg>()
            + sizes.notif as usize
            + sizes.notif_resp as usize;

        Self {
            proxy_msg: unsafe { mem::zeroed() },
            seccomp_notif: unsafe { mem::zeroed() },
            seccomp_resp: unsafe { mem::zeroed() },
            cookie_buf: unsafe { super::tools::vec::uninitialized(max_cookie) },
            sizes,
            seccomp_packet_size,
            pid_fd: None,
            mem_fd: None,
        }
    }

    fn reset(&mut self) {
        self.proxy_msg.cookie_len = 0;
        self.mem_fd = None;
        self.pid_fd = None;
    }

    /// Returns false on EOF.
    pub async fn recv(&mut self, socket: &SeqPacketSocket) -> Result<bool, Error> {
        // prepare buffers:
        self.reset();

        unsafe {
            self.cookie_buf.set_len(self.cookie_buf.capacity());
        }

        let mut iovec = [
            unsafe { io_vec_mut(&mut self.proxy_msg) },
            unsafe { io_vec_mut(&mut self.seccomp_notif) },
            unsafe { io_vec_mut(&mut self.seccomp_resp) },
            IoVecMut::new(self.cookie_buf.as_mut_slice()),
        ];

        unsafe {
            self.cookie_buf.set_len(0);
        }

        // receive:
        let mut fd_cmsg_buf = cmsg::buffer::<[RawFd; 2]>();
        let (datalen, cmsglen) = socket
            .recvmsg_vectored(&mut iovec, &mut fd_cmsg_buf)
            .await?;

        if datalen == 0 {
            return Ok(false);
        }

        self.set_len(datalen)?;

        // iterate through control messages:

        let cmsg = cmsg::iter(&fd_cmsg_buf[..cmsglen])
            .next()
            .ok_or_else(|| format_err!("missing file descriptors in message"))?;

        if cmsg.cmsg_level != libc::SOL_SOCKET && cmsg.cmsg_type != libc::SCM_RIGHTS {
            bail!("expected SCM_RIGHTS control message");
        }

        let fds: Vec<Fd> = cmsg
            .data
            .chunks_exact(mem::size_of::<RawFd>())
            .map(|chunk| unsafe {
                // clippy bug
                #[allow(clippy::cast_ptr_alignment)]
                Fd::from_raw_fd(std::ptr::read_unaligned(chunk.as_ptr() as _))
            })
            .collect();

        if fds.len() != 2 {
            bail!("expected exactly 2 file descriptors in control message");
        }

        let mut fds = fds.into_iter();
        let pid_fd = unsafe {
            PidFd::try_from_fd(
                fds.next()
                    .ok_or_else(|| format_err!("lxc seccomp message without pidfd"))?,
            )?
        };
        let mem_fd = fds
            .next()
            .ok_or_else(|| format_err!("lxc seccomp message without memfd"))?;

        self.pid_fd = Some(pid_fd);
        self.mem_fd = Some(std::fs::File::from_fd(mem_fd));

        Ok(true)
    }

    /// Get the process' pidfd.
    ///
    /// Note that the message must be valid, otherwise this panics!
    pub fn pid_fd(&self) -> &PidFd {
        self.pid_fd.as_ref().unwrap()
    }

    /// Get the process' mem fd.
    ///
    /// Note that this returns a non-mut trait object. This is because positional I/O does not need
    /// mutable self and the standard library correctly represents this in its `FileExt` trait!
    ///
    /// Note that the message must be valid, otherwise this panics!
    pub fn mem_fd(&self) -> &dyn FileExt {
        self.mem_fd.as_ref().unwrap()
    }

    /// Send the current data as response.
    pub async fn respond(&mut self, socket: &SeqPacketSocket) -> io::Result<()> {
        let iov = [
            unsafe { io_vec(&self.proxy_msg) },
            unsafe { io_vec(&self.seccomp_notif) },
            unsafe { io_vec(&self.seccomp_resp) },
        ];
        let len = iov.iter().map(|e| e.len()).sum();
        if socket.sendmsg_vectored(&iov).await? != len {
            io_bail!("truncated message?");
        }
        Ok(())
    }

    #[inline]
    fn prepare_response(&mut self) {
        let id = self.request().id;
        let resp = self.response_mut();
        resp.id = id;
        resp.val = -1;
        resp.error = -libc::ENOSYS;
        resp.flags = 0;
    }

    /// Called by recv() after the callback returned the new size. This verifies that there's
    /// enough data available.
    fn set_len(&mut self, len: usize) -> Result<(), Error> {
        if len < self.seccomp_packet_size {
            bail!("seccomp proxy message too short");
        }

        if self.proxy_msg.reserved0 != 0 {
            bail!("reserved data wasn't 0, liblxc secocmp notify protocol mismatch");
        }

        if !self.check_sizes() {
            bail!("seccomp proxy message content size validation failed");
        }

        if len - self.seccomp_packet_size > self.cookie_buf.capacity() {
            bail!("seccomp proxy message too long");
        }

        let cookie_len = match usize::try_from(self.proxy_msg.cookie_len) {
            Ok(cl) => cl,
            Err(_) => {
                self.proxy_msg.cookie_len = 0;
                bail!("cookie length exceeds our size type!");
            }
        };

        if len != self.seccomp_packet_size + cookie_len {
            bail!(
                "seccomp proxy packet contains unexpected cookie length {} + {} != {}",
                self.seccomp_packet_size,
                cookie_len,
                len
            );
        }

        unsafe {
            self.cookie_buf.set_len(cookie_len);
        }

        self.prepare_response();

        Ok(())
    }

    fn check_sizes(&self) -> bool {
        let got = self.proxy_msg.sizes.clone();
        got.notif == self.sizes.notif
            && got.notif_resp == self.sizes.notif_resp
            && got.data == self.sizes.data
    }

    /// Get the monitor pid from the current message.
    ///
    /// There's no guarantee that the pid is valid.
    #[inline]
    pub fn monitor_pid(&self) -> pid_t {
        self.proxy_msg.monitor_pid
    }

    /// Get the container's init pid from the current message.
    ///
    /// There's no guarantee that the pid is valid.
    #[inline]
    pub fn init_pid(&self) -> pid_t {
        self.proxy_msg.init_pid
    }

    /// Get the syscall request structure of this message.
    #[inline]
    pub fn request(&self) -> &SeccompNotif {
        &self.seccomp_notif
    }

    /// Access the response buffer of this message.
    #[inline]
    pub fn response_mut(&mut self) -> &mut SeccompNotifResp {
        &mut self.seccomp_resp
    }

    /// Get the cookie's length.
    #[inline]
    pub fn cookie_len(&self) -> usize {
        usize::try_from(self.proxy_msg.cookie_len).expect("cookie size should fit in an usize")
    }

    /// Get the cookie sent along with this message.
    #[inline]
    pub fn cookie(&self) -> &[u8] {
        &self.cookie_buf
    }

    /// Shortcut to get a parameter value.
    #[inline]
    fn arg(&self, arg: u32) -> Result<u64, Error> {
        self.request()
            .data
            .args
            .get(arg as usize)
            .copied()
            .ok_or_else(|| nix::errno::Errno::ERANGE.into())
    }

    /// Get a parameter as C String where the pointer may be `NULL`.
    ///
    /// Strings are limited to 4k bytes currently.
    #[inline]
    pub fn arg_opt_c_string(&self, arg: u32) -> Result<Option<CString>, Error> {
        let offset = self.arg(arg)?;
        if offset == 0 {
            Ok(None)
        } else {
            Ok(Some(crate::syscall::get_c_string(self, offset)?))
        }
    }

    /// Get a parameter as C String.
    ///
    /// Strings are limited to 4k bytes currently.
    #[inline]
    pub fn arg_c_string(&self, arg: u32) -> Result<CString, Error> {
        self.arg_opt_c_string(arg)?
            .ok_or_else(|| Errno::EINVAL.into())
    }

    /// Read a user space pointer parameter.
    #[inline]
    pub fn arg_struct_by_ptr<T>(&self, arg: u32) -> Result<T, Error> {
        let offset = self.arg(arg)?;
        let mut data: T = unsafe { mem::zeroed() };
        let slice = unsafe {
            std::slice::from_raw_parts_mut(&mut data as *mut _ as *mut u8, mem::size_of::<T>())
        };
        let got = self.mem_fd().read_at(slice, offset)?;
        if got != mem::size_of::<T>() {
            Err(Errno::EINVAL.into())
        } else {
            Ok(data)
        }
    }

    /// Read a user space pointer parameter.
    #[inline]
    pub fn mem_write_struct<T>(&self, offset: u64, data: &T) -> io::Result<()> {
        let slice = unsafe {
            std::slice::from_raw_parts(data as *const T as *const u8, mem::size_of::<T>())
        };
        let got = self.mem_fd().write_at(slice, offset)?;
        if got != mem::size_of::<T>() {
            Err(Errno::EINVAL.into())
        } else {
            Ok(())
        }
    }

    /// Checked way to get a `mode_t` argument.
    #[inline]
    pub fn arg_mode_t(&self, arg: u32) -> Result<nix::sys::stat::mode_t, Error> {
        nix::sys::stat::mode_t::try_from(self.arg(arg)?).map_err(|_| Error::from(Errno::EINVAL))
    }

    /// Checked way to get a `dev_t` argument.
    #[inline]
    pub fn arg_dev_t(&self, arg: u32) -> Result<nix::sys::stat::dev_t, Error> {
        self.arg(arg)
    }

    /// Checked way to get a file descriptor argument.
    #[inline]
    pub fn arg_fd(&self, arg: u32, flags: c_int) -> Result<Fd, Error> {
        let fd = RawFd::try_from(self.arg(arg)?).map_err(|_| Error::from(Errno::EINVAL))?;
        if fd == libc::AT_FDCWD {
            Ok(self.pid_fd().fd_cwd()?)
        } else {
            Ok(self.pid_fd().fd_num(fd, flags)?)
        }
    }

    /// Checked way to get a c_uint argument.
    #[inline]
    pub fn arg_uint(&self, arg: u32) -> Result<c_uint, Error> {
        c_uint::try_from(self.arg(arg)?).map_err(|_| Errno::EINVAL.into())
    }

    /// Checked way to get a c_int argument.
    #[inline]
    pub fn arg_int(&self, arg: u32) -> Result<c_int, Error> {
        self.arg_uint(arg).map(|u| u as c_int)
    }

    /// Checked way to get a `caddr_t` argument.
    #[inline]
    pub fn arg_caddr_t(&self, arg: u32) -> Result<*mut i8, Error> {
        Ok(self.arg(arg)? as *mut i8)
    }

    /// Checked way to get a raw pointer argument
    #[inline]
    pub fn arg_pointer(&self, arg: u32) -> Result<*const u8, Error> {
        Ok(self.arg(arg)? as usize as *const u8)
    }

    /// Checked way to get a raw char pointer.
    #[inline]
    pub fn arg_char_ptr(&self, arg: u32) -> Result<*const libc::c_char, Error> {
        Ok(self.arg(arg)? as usize as *const libc::c_char)
    }
}
