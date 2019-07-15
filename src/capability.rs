use std::io;
use std::os::raw::c_ulong;

use crate::{c_call, io_format_err};

bitflags::bitflags! {
    pub struct SecureBits: c_ulong {
        const NOROOT                        = 0b000000001;
        const NOROOT_LOCKED                 = 0b000000010;
        const NO_SETUID_FIXUP               = 0b000000100;
        const NO_SETUID_FIXUP_LOCKED        = 0b000001000;
        const KEEP_CAPS                     = 0b000010000;
        const KEEP_CAPS_LOCKED              = 0b000100000;
        const NO_CAP_AMBIENT_RAISE          = 0b001000000;
        const NO_CAP_AMBIENT_RAISE_LOCKED   = 0b010000000;

        const ALL_BITS                      = 0b001010101;
        const ALL_LOCKS                     = 0b010101010;
    }
}

impl SecureBits {
    pub fn apply(&self) -> io::Result<()> {
        c_call!(unsafe { libc::prctl(libc::PR_SET_SECUREBITS, self.bits()) })?;
        Ok(())
    }

    pub fn get_current() -> io::Result<Self> {
        let bits = c_call!(unsafe { libc::prctl(libc::PR_GET_SECUREBITS) })?;
        Self::from_bits(bits as _)
            .ok_or_else(|| io_format_err!("prctl() returned unknown securebits"))
    }
}
