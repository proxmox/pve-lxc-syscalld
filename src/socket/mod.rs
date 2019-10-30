use crate::rw_traits;

pub struct Pipe<RW> {
    fd: PolledFd,
}
