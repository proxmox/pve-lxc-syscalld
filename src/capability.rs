use std::io;
use std::os::raw::{c_int, c_ulong};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
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
        c_result!(unsafe { libc::prctl(libc::PR_SET_SECUREBITS, self.bits()) })?;
        Ok(())
    }

    pub fn get_current() -> io::Result<Self> {
        let bits = c_result!(unsafe { libc::prctl(libc::PR_GET_SECUREBITS) })?;
        Self::from_bits(bits as _)
            .ok_or_else(|| io_format_err!("prctl() returned unknown securebits"))
    }
}

#[derive(Clone, Default)]
pub struct Capabilities {
    pub inheritable: u64,
    pub permitted: u64,
    pub effective: u64,
    //bounding: u64, // we don't care currently
}

// Too lazy to bindgen libcap stuff...
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// Represents process capabilities.
///
/// This can be used to change the process' capability sets (if permitted by the kernel).
impl Capabilities {
    // We currently don't implement capget as it takes a pid which is racy on kernels without pidfd
    // support. Later on we might support a `capget(&PidFd)` method?

    /// Change our process capabilities. This does not include the bounding set.
    pub fn capset(&self) -> io::Result<()> {
        // kernel abi:
        #[allow(dead_code)]
        struct Header {
            version: u32,
            pid: c_int,
        }

        #[allow(dead_code)]
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

        c_try!(unsafe { libc::syscall(libc::SYS_capset, &header, &data) });

        Ok(())
    }
}
