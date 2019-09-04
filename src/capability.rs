use std::io;
use std::os::raw::c_ulong;

use crate::{c_call, io_format_err};

bitflags::bitflags! {
    pub struct SecureBits: c_ulong {
        const NOROOT                        = 0b0_0000_0001;
        const NOROOT_LOCKED                 = 0b0_0000_0010;
        const NO_SETUID_FIXUP               = 0b0_0000_0100;
        const NO_SETUID_FIXUP_LOCKED        = 0b0_0000_1000;
        const KEEP_CAPS                     = 0b0_0001_0000;
        const KEEP_CAPS_LOCKED              = 0b0_0010_0000;
        const NO_CAP_AMBIENT_RAISE          = 0b0_0100_0000;
        const NO_CAP_AMBIENT_RAISE_LOCKED   = 0b0_1000_0000;

        const ALL_BITS                      = 0b0_0101_0101;
        const ALL_LOCKS                     = 0b0_1010_1010;
    }
}

impl SecureBits {
    pub fn apply(self) -> io::Result<()> {
        c_call!(unsafe { libc::prctl(libc::PR_SET_SECUREBITS, self.bits()) })?;
        Ok(())
    }

    pub fn get_current() -> io::Result<Self> {
        let bits = c_call!(unsafe { libc::prctl(libc::PR_GET_SECUREBITS) })?;
        Self::from_bits(bits as _)
            .ok_or_else(|| io_format_err!("prctl() returned unknown securebits"))
    }
}
