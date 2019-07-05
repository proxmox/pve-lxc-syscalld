//! Module for LXC specific related seccomp handling.

use std::convert::TryFrom;
use std::{io, mem};

use failure::{bail, Error};
use libc::pid_t;

use super::seccomp::{SeccompNotif, SeccompNotifResp, SeccompNotifSizes};

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
#[repr(C)]
pub struct ProxyMessageBuffer {
    buffer: Vec<u8>,
    sizes: SeccompNotifSizes,
    seccomp_packet_size: usize,
}

impl ProxyMessageBuffer {
    /// Allocate a new proxy message buffer with a specific maximum cookie size.
    pub fn new(max_cookie: usize) -> io::Result<Self> {
        let sizes = SeccompNotifSizes::get()?;
        let max_size = sizes.notif as usize + sizes.notif_resp as usize + max_cookie;
        let seccomp_packet_size = mem::size_of::<SeccompNotifyProxyMsg>()
            + sizes.notif as usize
            + sizes.notif_resp as usize;
        Ok(Self {
            buffer: unsafe { super::tools::vec::uninitialized(max_size) },
            sizes,
            seccomp_packet_size,
        })
    }

    /// Allow this buffer to be filled with new data.
    ///
    /// This resets the buffer's length to its full capacity and returns a mutable slice.
    ///
    /// After this you must call `set_len()` with the number of bytes written to the buffer to
    /// verify the new contents.
    pub unsafe fn new_mut(&mut self) -> &mut [u8] {
        self.buffer.set_len(self.buffer.capacity());
        &mut self.buffer[..]
    }

    fn drop_cookie(&mut self) {
        self.msg_mut().cookie_len = 0;
        unsafe {
            self.buffer.set_len(self.seccomp_packet_size);
        }
    }

    /// Prepare to send a reply.
    ///
    /// This drops the cookie and returns a byte slice of the proxy message struct suitable to be
    /// sent as a response to the lxc monitor.
    ///
    /// The cookie will be inaccessible after this.
    pub fn as_buf_no_cookie(&mut self) -> &[u8] {
        self.drop_cookie();
        &self.buffer[..]
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

    /// You must call this after writing a new packet to via `new_mut()`. This verifies that there's
    /// enough data available.
    ///
    /// If this returns false, you must not attempt to access the data!
    pub fn set_len(&mut self, len: usize) -> Result<(), Error> {
        if len > self.buffer.capacity() {
            bail!("seccomp proxy message longer than buffer capacity");
        }

        if !self.validate() {
            bail!("seccomp proxy message content size validation failed");
        }

        if len != self.seccomp_packet_size + self.cookie_len() {
            bail!(
                "seccomp proxy packet contains unexpected cookie length {} + {} != {}",
                self.seccomp_packet_size,
                self.cookie_len(),
                len
            );
        }

        unsafe {
            self.buffer.set_len(len);
        }

        self.prepare_response();

        Ok(())
    }

    fn validate(&self) -> bool {
        if self.reserved0() != 0 {
            return false;
        }

        let got = self.msg().sizes.clone();
        got.notif == self.sizes.notif
            && got.notif_resp == self.sizes.notif_resp
            && got.data == self.sizes.data
    }

    #[inline]
    fn msg_ptr(&self) -> *const SeccompNotifyProxyMsg {
        self.buffer.as_ptr() as *const SeccompNotifyProxyMsg
    }

    #[inline]
    fn msg(&self) -> &SeccompNotifyProxyMsg {
        unsafe { &*self.msg_ptr() }
    }

    #[inline]
    fn msg_mut_ptr(&mut self) -> *mut SeccompNotifyProxyMsg {
        self.buffer.as_mut_ptr() as *mut SeccompNotifyProxyMsg
    }

    #[inline]
    fn msg_mut(&mut self) -> &mut SeccompNotifyProxyMsg {
        unsafe { &mut *self.msg_mut_ptr() }
    }

    fn reserved0(&self) -> u64 {
        self.msg().reserved0
    }

    /// Get the monitor pid from the current message.
    ///
    /// There's no guarantee that the pid is valid.
    pub fn monitor_pid(&self) -> pid_t {
        self.msg().monitor_pid
    }

    /// Get the container's init pid from the current message.
    ///
    /// There's no guarantee that the pid is valid.
    pub fn init_pid(&self) -> pid_t {
        self.msg().init_pid
    }

    /// Get the syscall request structure of this message.
    pub fn request(&self) -> &SeccompNotif {
        unsafe {
            &*(self
                .buffer
                .as_ptr()
                .add(mem::size_of::<SeccompNotifyProxyMsg>()) as *const SeccompNotif)
        }
    }

    /// Access the response buffer of this message.
    pub fn response_mut(&mut self) -> &mut SeccompNotifResp {
        unsafe {
            &mut *(self
                .buffer
                .as_mut_ptr()
                .add(mem::size_of::<SeccompNotifyProxyMsg>())
                .add(usize::from(self.sizes.notif)) as *mut SeccompNotifResp)
        }
    }

    /// Get the cookie's length.
    pub fn cookie_len(&self) -> usize {
        usize::try_from(self.msg().cookie_len).expect("cookie size should fit in an usize")
    }

    /// Get the cookie sent along with this message.
    pub fn cookie(&self) -> &[u8] {
        let len = self.cookie_len();
        unsafe {
            let start = self
                .buffer
                .as_ptr()
                .add(mem::size_of::<SeccompNotifyProxyMsg>())
                .add(usize::from(self.sizes.notif))
                .add(usize::from(self.sizes.notif_resp));

            std::slice::from_raw_parts(start, len)
        }
    }
}
