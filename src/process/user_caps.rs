//! User and capability management.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;

use anyhow::Error;

use super::PidFd;
use crate::capability::Capabilities;

/// Helper to enter a process' permission-check environment.
///
/// When we execute a syscall on behalf of another process, we should try to trigger as many
/// permission checks as we can. It is impractical to implement them all manually, so the best
/// thing to do is cause as many of them to happen on the kernel-side as we can.
///
/// We start by entering the process' devices and v2 cgroup. As calls like `mknod()` may be
/// affected, and access to devices as well.
///
/// Then we must enter the mount namespace, chroot and current working directory, in order to get
/// the correct view of paths.
///
/// Next we copy the caller's `umask`.
///
/// Then switch over our effective and file system uid and gid. This has 2 reasons: First, it means
/// we do not need to run `chown()` on files we create, secondly, the user may have dropped
/// `CAP_DAC_OVERRIDE` / `CAP_DAC_READ_SEARCH` which may have prevented the creation of the file in
/// the first place (for example, the container program may be a non-root executable with
/// `cap_mknod=ep` as file-capabilities, in which case we do not want a user to be allowed to run
/// `mknod()` on a path owned by different user (and checking file system permissions would
/// require us to handle ACLs, quotas, which are all file system tyep dependent as well, so better
/// leave all that up to the kernel, too!)).
///
/// Next we clone the process' capability set. This is because the process may have dropped
/// capabilties which under normal conditions would prevent them from executing the syscall.  For
/// example a process may be executing `mknod()` after having dropped `CAP_MKNOD`.
#[derive(Clone)]
#[must_use = "not using UserCaps may be a security issue"]
pub struct UserCaps<'a> {
    pidfd: &'a PidFd,
    apply_uids: bool,
    euid: libc::uid_t,
    egid: libc::gid_t,
    fsuid: libc::uid_t,
    fsgid: libc::gid_t,
    capabilities: Capabilities,
    umask: libc::mode_t,
    cgroup_v1_devices: Option<OsString>,
    cgroup_v2_base: &'static str,
    cgroup_v2: Option<OsString>,
    apparmor_profile: Option<OsString>,
}

impl UserCaps<'_> {
    pub fn new(pidfd: &PidFd) -> Result<UserCaps, Error> {
        let status = pidfd.get_status()?;
        let cgroups = pidfd.get_cgroups()?;
        let apparmor_profile = crate::apparmor::get_label(pidfd)?;

        Ok(UserCaps {
            pidfd,
            apply_uids: true,
            euid: status.uids.euid,
            egid: status.uids.egid,
            fsuid: status.uids.fsuid,
            fsgid: status.uids.fsgid,
            capabilities: status.capabilities,
            umask: status.umask,
            cgroup_v1_devices: cgroups.get("devices").map(|s| s.to_owned()),
            cgroup_v2_base: if cgroups.has_v1() { "unified/" } else { "" },
            cgroup_v2: cgroups.v2().map(|s| s.to_owned()),
            apparmor_profile,
        })
    }

    fn apply_cgroups(&self) -> io::Result<()> {
        // FIXME: Handle `kind` taking /proc/self/mountinfo into account instead of assuming
        // "unified/"
        fn enter_cgroup(kind: &str, name: &OsStr) -> io::Result<()> {
            let mut path = OsString::with_capacity(15 + kind.len() + name.len() + 13 + 1);
            path.push(OsStr::from_bytes(b"/sys/fs/cgroup/"));
            path.push(kind);
            path.push(name);
            path.push(OsStr::from_bytes(b"/cgroup.procs"));
            std::fs::write(path, b"0")
        }

        if let Some(ref cg) = self.cgroup_v1_devices {
            enter_cgroup("devices/", cg)?;
        }

        if let Some(ref cg) = self.cgroup_v2 {
            enter_cgroup(self.cgroup_v2_base, cg)?;
        }

        Ok(())
    }

    fn apply_user_caps(&self) -> io::Result<()> {
        use crate::capability::SecureBits;
        if self.apply_uids {
            unsafe {
                libc::umask(self.umask);
            }
            let mut secbits = SecureBits::get_current()?;
            secbits |= SecureBits::KEEP_CAPS | SecureBits::NO_SETUID_FIXUP;
            secbits.apply()?;
            c_try!(unsafe { libc::setegid(self.egid) });
            c_try!(unsafe { libc::setfsgid(self.fsgid) });
            c_try!(unsafe { libc::seteuid(self.euid) });
            c_try!(unsafe { libc::setfsuid(self.fsuid) });
        }
        self.capabilities.capset()?;
        Ok(())
    }

    pub fn disable_uid_change(&mut self) {
        self.apply_uids = false;
    }

    pub fn disable_cgroup_change(&mut self) {
        self.cgroup_v1_devices = None;
        self.cgroup_v2 = None;
    }

    pub fn apply(self, own_pidfd: &PidFd) -> io::Result<()> {
        self.apply_cgroups()?;
        self.pidfd.mount_namespace()?.setns()?;
        self.pidfd.enter_chroot()?;
        self.pidfd.enter_cwd()?;
        if let Some(ref label) = self.apparmor_profile {
            crate::apparmor::set_label(own_pidfd, label)?;
        }
        self.apply_user_caps()?;
        Ok(())
    }
}
