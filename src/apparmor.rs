//! AppArmor utility functions.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use crate::process::PidFd;

pub fn get_label(pidfd: &PidFd) -> io::Result<Option<OsString>> {
    let mut out = match pidfd.read_file(c_str!("attr/current")) {
        Ok(out) => out,
        Err(ref e) if e.raw_os_error() == Some(libc::EINVAL) => return Ok(None),
        Err(other) => return Err(other),
    };

    if out.is_empty() {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }

    if let Some(pos) = out.iter().position(|c| *c == b' ' || *c == b'\n') {
        out.truncate(pos);
    }

    Ok(Some(OsString::from_vec(out)))
}

pub fn set_label(pidfd: &PidFd, label: &OsStr) -> io::Result<()> {
    let mut file = pidfd.open_file(c_str!("attr/current"), libc::O_RDWR | libc::O_CLOEXEC, 0)?;

    let mut bytes = Vec::with_capacity(14 + label.len());
    bytes.extend_from_slice(b"changeprofile ");
    bytes.extend_from_slice(label.as_bytes());

    file.write_all(&bytes)?;
    Ok(())
}
