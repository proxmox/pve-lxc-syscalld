use crate::capability::Capabilities;

pub mod cgroups;
pub mod id_map;
pub mod pid_fd;
pub mod user_caps;

#[doc(inline)]
pub use cgroups::CGroups;

#[doc(inline)]
pub use pid_fd::PidFd;

#[doc(inline)]
pub use id_map::{IdMap, IdMapEntry};

#[doc(inline)]
pub use user_caps::UserCaps;

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

#[derive(Default)]
pub struct ProcStatus {
    uids: Uids,
    capabilities: Capabilities,
    umask: libc::mode_t,
}
