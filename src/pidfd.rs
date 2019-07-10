//! pidfd helper functionality

use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io::{self, BufRead, BufReader};
use std::os::raw::c_int;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use failure::{bail, Error};
use libc::pid_t;

use crate::libc_try;
use crate::nsfd::{ns_type, NsFd};
use crate::tools::Fd;

pub struct PidFd(RawFd, pid_t);
crate::file_descriptor_impl!(PidFd);

#[derive(Default)]
pub struct Uids {
    pub ruid: libc::uid_t,
    pub euid: libc::uid_t,
    pub suid: libc::uid_t,
    pub fsuid: libc::uid_t,
    pub rgid: libc::gid_t,
    pub egid: libc::gid_t,
    pub sgid: libc::gid_t,
    pub fsgid: libc::gid_t,
}

#[derive(Clone, Default)]
pub struct Capabilities {
    inheritable: u64,
    permitted: u64,
    effective: u64,
    //bounding: u64, // we don't care currently
}

#[derive(Default)]
pub struct ProcStatus {
    uids: Uids,
    capabilities: Capabilities,
    umask: libc::mode_t,
}

impl PidFd {
    pub fn open(pid: pid_t) -> io::Result<Self> {
        let path = CString::new(format!("/proc/{}", pid)).unwrap();

        let fd =
            libc_try!(unsafe { libc::open(path.as_ptr(), libc::O_DIRECTORY | libc::O_CLOEXEC) });

        Ok(Self(fd, pid))
    }

    pub unsafe fn try_from_fd(fd: Fd) -> io::Result<Self> {
        let mut this = Self(fd.into_raw_fd(), -1 as pid_t);
        let pid = this.read_pid()?;
        this.1 = pid;
        Ok(this)
    }

    pub fn mount_namespace(&self) -> io::Result<NsFd<ns_type::Mount>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/mnt\0")
        })
    }

    pub fn cgroup_namespace(&self) -> io::Result<NsFd<ns_type::Cgroup>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/cgroup\0")
        })
    }

    pub fn user_namespace(&self) -> io::Result<NsFd<ns_type::User>> {
        NsFd::openat(self.0, unsafe {
            CStr::from_bytes_with_nul_unchecked(b"ns/user\0")
        })
    }

    fn fd(&self, path: &CStr, flags: c_int, mode: c_int) -> io::Result<Fd> {
        Ok(Fd(libc_try!(unsafe {
            libc::openat(
                self.as_raw_fd(),
                path.as_ptr() as *const _,
                flags | libc::O_CLOEXEC,
                mode,
            )
        })))
    }

    pub fn fd_cwd(&self) -> io::Result<Fd> {
        self.fd(
            unsafe { CStr::from_bytes_with_nul_unchecked(b"cwd\0") },
            libc::O_DIRECTORY,
            0,
        )
    }

    pub fn fd_num(&self, num: RawFd, flags: c_int) -> io::Result<Fd> {
        let path = format!("fd/{}\0", num);
        self.fd(
            unsafe { CStr::from_bytes_with_nul_unchecked(path.as_bytes()) },
            flags,
            0,
        )
    }

    pub fn enter_cwd(&self) -> io::Result<()> {
        libc_try!(unsafe { libc::fchdir(self.fd_cwd()?.as_raw_fd()) });
        Ok(())
    }

    pub fn enter_chroot(&self) -> io::Result<()> {
        libc_try!(unsafe { libc::fchdir(self.as_raw_fd()) });
        libc_try!(unsafe { libc::chroot(b"root\0".as_ptr() as *const _) });
        libc_try!(unsafe { libc::chdir(b"/\0".as_ptr() as *const _) });
        Ok(())
    }

    // procfs files cannot be async, we cannot add them to epoll...
    pub fn open_file(&self, path: &CStr, flags: c_int, mode: c_int) -> io::Result<std::fs::File> {
        Ok(unsafe { std::fs::File::from_raw_fd(self.fd(path, flags, mode)?.into_raw_fd()) })
    }

    #[inline]
    fn open_buffered(&self, path: &CStr) -> io::Result<impl BufRead> {
        Ok(BufReader::new(self.open_file(
            path,
            libc::O_RDONLY | libc::O_CLOEXEC,
            0,
        )?))
    }

    #[inline]
    pub fn get_pid(&self) -> pid_t {
        self.1
    }

    fn read_pid(&self) -> io::Result<pid_t> {
        let reader =
            self.open_buffered(unsafe { CStr::from_bytes_with_nul_unchecked(b"status\0") })?;

        for line in reader.lines() {
            let line = line?;
            let mut parts = line.split_ascii_whitespace();
            if parts.next() == Some("Pid:") {
                let pid = parts
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "bad 'Pid:' line in proc"))?
                    .parse::<pid_t>()
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::Other, "failed to parse pid from proc")
                    })?;
                return Ok(pid);
            }
        }

        Err(io::ErrorKind::NotFound.into())
    }

    pub fn get_status(&self) -> io::Result<ProcStatus> {
        let reader =
            self.open_buffered(unsafe { CStr::from_bytes_with_nul_unchecked(b"status\0") })?;

        #[inline]
        fn check_uid_gid(value: Option<&str>) -> io::Result<libc::uid_t> {
            value
                .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "bad 'Uid/Gid:' line in proc"))?
                .parse::<libc::uid_t>()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "failed to parse uid from proc"))
        }

        #[inline]
        fn check_u64_hex(value: Option<&str>) -> io::Result<u64> {
            Ok(u64::from_str_radix(
                value.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "bad numeric property line in proc")
                })?,
                16,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?)
        }

        #[inline]
        fn check_u32_oct(value: Option<&str>) -> io::Result<u32> {
            Ok(u32::from_str_radix(
                value.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "bad numeric property line in proc")
                })?,
                8,
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?)
        }

        let mut ids = Uids::default();
        let mut caps = Capabilities::default();
        let mut umask = 0o022;
        for line in reader.lines() {
            let line = line?;
            let mut parts = line.split_ascii_whitespace();
            match parts.next() {
                Some("Uid:") => {
                    ids.ruid = check_uid_gid(parts.next())?;
                    ids.euid = check_uid_gid(parts.next())?;
                    ids.suid = check_uid_gid(parts.next())?;
                    ids.fsuid = check_uid_gid(parts.next())?;
                }
                Some("Gid:") => {
                    ids.rgid = check_uid_gid(parts.next())?;
                    ids.egid = check_uid_gid(parts.next())?;
                    ids.sgid = check_uid_gid(parts.next())?;
                    ids.fsgid = check_uid_gid(parts.next())?;
                }
                Some("CapInh:") => caps.inheritable = check_u64_hex(parts.next())?,
                Some("CapPrm:") => caps.permitted = check_u64_hex(parts.next())?,
                Some("CapEff:") => caps.effective = check_u64_hex(parts.next())?,
                //Some("CapBnd:") => caps.bounding = check_u64_hex(parts.next())?,
                Some("Umask:") => umask = check_u32_oct(parts.next())?,
                _ => continue,
            }
        }

        Ok(ProcStatus {
            uids: ids,
            capabilities: caps,
            umask,
        })
    }

    pub fn get_cgroups(&self) -> Result<CGroups, Error> {
        let reader =
            self.open_buffered(unsafe { CStr::from_bytes_with_nul_unchecked(b"cgroup\0") })?;

        let mut cgroups = CGroups::new();

        for line in reader.split(b'\n') {
            let line = line?;
            let mut parts = line.splitn(3, |b| *b == b':');
            let num = parts.next();
            let name = parts.next();
            let path = parts.next();
            if !num.is_some() || !name.is_some() || !path.is_some() || parts.next().is_some() {
                bail!("failed to parse cgroup line: {:?}", line);
            }

            let name = String::from_utf8(name.unwrap().to_vec())?;
            let path = OsString::from_vec(path.unwrap().to_vec());

            if name.len() == 0 {
                cgroups.v2 = Some(path);
            } else {
                for entry in name.split(',') {
                    cgroups.v1.insert(entry.to_string(), path.clone());
                }
            }
        }

        Ok(cgroups)
    }

    pub fn user_caps(&self) -> Result<UserCaps, Error> {
        UserCaps::new(self)
    }
}

pub struct CGroups {
    v1: HashMap<String, OsString>,
    v2: Option<OsString>,
}

impl CGroups {
    fn new() -> Self {
        Self {
            v1: HashMap::new(),
            v2: None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.v1.get(name).map(|s| s.as_os_str())
    }

    pub fn v2(&self) -> Option<&OsStr> {
        self.v2.as_ref().map(|s| s.as_os_str())
    }
}

// Too lazy to bindgen libcap stuff...
const CAPABILITY_VERSION_3: u32 = 0x20080522;

/// Represents process capabilities.
///
/// This can be used to change the process' capability sets (if permitted by the kernel).
impl Capabilities {
    // We currently don't implement capget as it takes a pid which is racy on kernels without pidfd
    // support. Later on we might support a `capget(&PidFd)` method?

    /// Change our process capabilities. This does not include the bounding set.
    pub fn capset(&self) -> io::Result<()> {
        #![allow(dead_code)]
        // kernel abi:
        struct Header {
            version: u32,
            pid: c_int,
        }

        struct Data {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }

        let header = Header {
            version: CAPABILITY_VERSION_3,
            pid: 0, // equivalent to gettid(),
        };

        let data = [
            Data {
                effective: self.effective as u32,
                permitted: self.permitted as u32,
                inheritable: self.inheritable as u32,
            },
            Data {
                effective: (self.effective >> 32) as u32,
                permitted: (self.permitted >> 32) as u32,
                inheritable: (self.inheritable >> 32) as u32,
            },
        ];

        libc_try!(unsafe { libc::syscall(libc::SYS_capset, &header, &data) });

        Ok(())
    }

    /// Change the thread's keep-capabilities flag.
    pub fn set_keep_caps(on: bool) -> io::Result<()> {
        libc_try!(unsafe { libc::prctl(libc::PR_SET_KEEPCAPS, c_int::from(on)) });
        Ok(())
    }
}

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
    euid: libc::uid_t,
    egid: libc::gid_t,
    fsuid: libc::uid_t,
    fsgid: libc::gid_t,
    capabilities: Capabilities,
    umask: libc::mode_t,
    cgroup_v1_devices: Option<OsString>,
    cgroup_v2: Option<OsString>,
}

impl UserCaps<'_> {
    pub fn new(pidfd: &PidFd) -> Result<UserCaps, Error> {
        let status = pidfd.get_status()?;
        let cgroups = pidfd.get_cgroups()?;

        Ok(UserCaps {
            pidfd,
            euid: status.uids.euid,
            egid: status.uids.egid,
            fsuid: status.uids.fsuid,
            fsgid: status.uids.fsgid,
            capabilities: status.capabilities,
            umask: status.umask,
            cgroup_v1_devices: cgroups.get("devices").map(|s| s.to_owned()),
            cgroup_v2: cgroups.v2().map(|s| s.to_owned()),
        })
    }

    fn apply_cgroups(&self) -> io::Result<()> {
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
            enter_cgroup("unified/", cg)?;
        }

        Ok(())
    }

    fn apply_user_caps(&self) -> io::Result<()> {
        unsafe {
            libc::umask(self.umask);
        }
        Capabilities::set_keep_caps(true)?;
        libc_try!(unsafe { libc::setegid(self.egid) });
        libc_try!(unsafe { libc::setfsgid(self.fsgid) });
        libc_try!(unsafe { libc::seteuid(self.euid) });
        libc_try!(unsafe { libc::setfsuid(self.fsuid) });
        self.capabilities.capset()?;
        Ok(())
    }

    pub fn apply(self) -> io::Result<()> {
        self.apply_cgroups()?;
        self.pidfd.mount_namespace()?.setns()?;
        self.pidfd.enter_chroot()?;
        self.pidfd.enter_cwd()?;
        self.apply_user_caps()?;
        Ok(())
    }
}
