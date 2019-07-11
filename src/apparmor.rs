//! AppArmor utility functions.

use std::ffi::{CStr, OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use crate::pidfd::PidFd;

pub fn get_label(pidfd: &PidFd) -> io::Result<Option<OsString>> {
    let mut out = match pidfd.read_file(unsafe {
        CStr::from_bytes_with_nul_unchecked(b"attr/current\0")
    }) {
        Ok(out) => out,
        Err(ref e) if e.raw_os_error() == Some(libc::EINVAL) => return Ok(None),
        Err(other) => return Err(other.into()),
    };

    if out.len() == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }

    if let Some(pos) = out.iter().position(|c| *c == b' ' || *c == b'\n') {
        out.truncate(pos);
    }

    Ok(Some(OsString::from_vec(out)))
}

pub fn set_label(pidfd: &PidFd, label: &OsStr) -> io::Result<()> {
    let mut file = pidfd.open_file(
        unsafe { CStr::from_bytes_with_nul_unchecked(b"attr/current\0") },
        libc::O_RDWR | libc::O_CLOEXEC,
        0
    )?;

    let mut bytes = Vec::with_capacity(14 + label.len());
    bytes.extend_from_slice(b"changeprofile ");
    bytes.extend_from_slice(label.as_bytes());

    file.write_all(&bytes)?;
    Ok(())
}
