//! pidfd helper functionality

use std::ffi::{CStr, CString, OsString};
use std::io::{self, BufRead, BufReader};
use std::os::raw::c_int;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use anyhow::{bail, Error};
use libc::pid_t;

use crate::capability::Capabilities;
use crate::error::io_err_other;
use crate::nsfd::{ns_type, NsFd};
use crate::tools::Fd;

use super::{CGroups, IdMap, IdMapEntry, ProcStatus, Uids, UserCaps};

pub struct PidFd(RawFd, pid_t);
file_descriptor_impl!(PidFd);

impl PidFd {
    pub fn current() -> io::Result<Self> {
        Self::open(unsafe { libc::getpid() })
    }

    pub fn open(pid: pid_t) -> io::Result<Self> {
        let path = CString::new(format!("/proc/{}", pid)).unwrap();

        let fd = c_try!(unsafe { libc::open(path.as_ptr(), libc::O_DIRECTORY | libc::O_CLOEXEC) });

        Ok(Self(fd, pid))
    }

    /// Turn a valid pid file descriptor into a PidFd.
    ///
    /// # Safety
    ///
    /// The file descriptor must already be a valid pidfd, this is not checked. This function only
    /// fails if reading the pid from the pidfd's proc entry fails.
    pub unsafe fn try_from_fd(fd: Fd) -> io::Result<Self> {
        #[allow(clippy::unnecessary_cast)] // pid_t is a type alias
        let mut this = Self(fd.into_raw_fd(), -1 as pid_t);
        let pid = this.read_pid()?;
        this.1 = pid;
        Ok(this)
    }

    pub fn mount_namespace(&self) -> io::Result<NsFd<ns_type::Mount>> {
        NsFd::openat(self.0, c_str!("ns/mnt"))
    }

    pub fn cgroup_namespace(&self) -> io::Result<NsFd<ns_type::Cgroup>> {
        NsFd::openat(self.0, c_str!("ns/cgroup"))
    }

    pub fn user_namespace(&self) -> io::Result<NsFd<ns_type::User>> {
        NsFd::openat(self.0, c_str!("ns/user"))
    }

    fn fd(&self, path: &CStr, flags: c_int, mode: c_int) -> io::Result<Fd> {
        Ok(Fd(c_try!(unsafe {
            libc::openat(
                self.as_raw_fd(),
                path.as_ptr() as *const _,
                flags | libc::O_CLOEXEC,
                mode,
            )
        })))
    }

    pub fn fd_cwd(&self) -> io::Result<Fd> {
        self.fd(c_str!("cwd"), libc::O_DIRECTORY, 0)
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
        c_try!(unsafe { libc::fchdir(self.fd_cwd()?.as_raw_fd()) });
        Ok(())
    }

    pub fn enter_chroot(&self) -> io::Result<()> {
        c_try!(unsafe { libc::fchdir(self.as_raw_fd()) });
        c_try!(unsafe { libc::chroot(b"root\0".as_ptr() as *const _) });
        c_try!(unsafe { libc::chdir(b"/\0".as_ptr() as *const _) });
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
        let reader = self.open_buffered(c_str!("status"))?;

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

    #[inline]
    fn __check_uid_gid(value: Option<&str>) -> io::Result<libc::uid_t> {
        value
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "bad 'Uid/Gid:' line in proc"))?
            .parse::<libc::uid_t>()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "failed to parse uid from proc"))
    }

    pub fn get_status(&self) -> io::Result<ProcStatus> {
        let reader = self.open_buffered(c_str!("status"))?;

        #[inline]
        fn check_u64_hex(value: Option<&str>) -> io::Result<u64> {
            u64::from_str_radix(
                value.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "bad numeric property line in proc")
                })?,
                16,
            )
            .map_err(io_err_other)
        }

        #[inline]
        fn check_u32_oct(value: Option<&str>) -> io::Result<u32> {
            u32::from_str_radix(
                value.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "bad numeric property line in proc")
                })?,
                8,
            )
            .map_err(io_err_other)
        }

        let mut ids = Uids::default();
        let mut caps = Capabilities::default();
        let mut umask = 0o022;
        for line in reader.lines() {
            let line = line?;
            let mut parts = line.split_ascii_whitespace();
            match parts.next() {
                Some("Uid:") => {
                    ids.ruid = Self::__check_uid_gid(parts.next())?;
                    ids.euid = Self::__check_uid_gid(parts.next())?;
                    ids.suid = Self::__check_uid_gid(parts.next())?;
                    ids.fsuid = Self::__check_uid_gid(parts.next())?;
                }
                Some("Gid:") => {
                    ids.rgid = Self::__check_uid_gid(parts.next())?;
                    ids.egid = Self::__check_uid_gid(parts.next())?;
                    ids.sgid = Self::__check_uid_gid(parts.next())?;
                    ids.fsgid = Self::__check_uid_gid(parts.next())?;
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
        let reader = self.open_buffered(c_str!("cgroup"))?;

        let mut cgroups = CGroups::new();

        for line in reader.split(b'\n') {
            let line = line?;
            let mut parts = line.splitn(3, |b| *b == b':');
            let num = parts.next();
            let name = parts.next();
            let path = parts.next();
            if num.is_none() || name.is_none() || path.is_none() || parts.next().is_some() {
                bail!("failed to parse cgroup line: {:?}", line);
            }

            let name = String::from_utf8(name.unwrap().to_vec())?;
            let path = OsString::from_vec(path.unwrap().to_vec());

            if name.is_empty() {
                cgroups.v2 = Some(path);
            } else {
                for entry in name.split(',') {
                    cgroups
                        .v1
                        .get_or_insert_with(Default::default)
                        .insert(entry.to_string(), path.clone());
                }
            }
        }

        Ok(cgroups)
    }

    pub fn get_uid_gid_map(&self, file: &CStr) -> Result<IdMap, Error> {
        let reader = self.open_buffered(file)?;

        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let mut parts = line.split_ascii_whitespace();
            let ns = u64::from(Self::__check_uid_gid(parts.next())?);
            let host = u64::from(Self::__check_uid_gid(parts.next())?);
            let range = u64::from(Self::__check_uid_gid(parts.next())?);
            entries.push(IdMapEntry { ns, host, range });
        }

        Ok(IdMap::new(entries))
    }

    pub fn get_uid_map(&self) -> Result<IdMap, Error> {
        self.get_uid_gid_map(c_str!("uid_map"))
    }

    pub fn get_gid_map(&self) -> Result<IdMap, Error> {
        self.get_uid_gid_map(c_str!("gid_map"))
    }

    pub fn read_file(&self, file: &CStr) -> io::Result<Vec<u8>> {
        use io::Read;

        let mut reader = self.open_file(file, libc::O_RDONLY | libc::O_CLOEXEC, 0)?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out)?;
        Ok(out)
    }

    pub fn user_caps(&self) -> Result<UserCaps, Error> {
        UserCaps::new(self)
    }
}
