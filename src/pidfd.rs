//! pidfd helper functionality

use std::ffi::{CStr, CString};
use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use crate::nsfd::{ns_type, NsFd};
use crate::tools::Fd;
use crate::{file_descriptor_type, libc_try};

file_descriptor_type!(PidFd);

impl PidFd {
    pub fn open(pid: libc::pid_t) -> io::Result<Self> {
        let path = CString::new(format!("/proc/{}", pid)).unwrap();

        let fd =
            libc_try!(unsafe { libc::open(path.as_ptr(), libc::O_DIRECTORY | libc::O_CLOEXEC) });

        Ok(Self(fd))
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
        self.fd(unsafe { CStr::from_bytes_with_nul_unchecked(b"cwd\0") }, libc::O_DIRECTORY, 0)
    }

    pub fn fd_num(&self, num: RawFd, flags: c_int) -> io::Result<Fd> {
        let path = format!("fd/{}\0", num);
        self.fd(unsafe { CStr::from_bytes_with_nul_unchecked(path.as_bytes()) }, flags, 0)
    }

    //pub fn dup(&self) -> io::Result<Self> {
    //    Ok(Self(libc_try!(unsafe {
    //        libc::fcntl(self.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0)
    //    })))
    //}


    // procfs files cannot be async, we cannot add them to epoll...
    pub fn open_file(&self, path: &CStr, flags: c_int, mode: c_int) -> io::Result<std::fs::File> {
        Ok(unsafe { std::fs::File::from_raw_fd(self.fd(path, flags, mode)?.into_raw_fd()) })
    }

    pub fn get_euid_egid(&self) -> io::Result<(libc::uid_t, libc::gid_t)> {
        use io::BufRead;

        let reader = io::BufReader::new(self.open_file(
            unsafe { CStr::from_bytes_with_nul_unchecked(b"status\0") },
            libc::O_RDONLY | libc::O_CLOEXEC,
            0,
        )?);

        let mut uid = None;
        let mut gid = None;
        for line in reader.lines() {
            let line = line?;
            let mut parts = line.split_ascii_whitespace();
            match parts.next() {
                Some("Uid:") => {
                    uid = Some(parts
                        .skip(1)
                        .next()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::Other, "bad 'Uid:' line in proc")
                        })?
                        .parse::<libc::uid_t>()
                        .map_err(|_| {
                            io::Error::new(io::ErrorKind::Other, "failed to parse uid from proc")
                        })?
                    );
                }
                Some("Gid:") => {
                    gid = Some(parts
                        .skip(1)
                        .next()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::Other, "bad 'Uid:' line in proc")
                        })?
                        .parse::<libc::gid_t>()
                        .map_err(|_| {
                            io::Error::new(io::ErrorKind::Other, "failed to parse gid from proc")
                        })?
                    );
                }
                _ => continue,
            }
            if let (Some(u), Some(g)) = (uid, gid) {
                return Ok((u, g));
            }
        }

        Err(io::ErrorKind::InvalidData.into())
    }
}
