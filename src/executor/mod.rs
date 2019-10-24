use std::io;

pub mod thread_pool;

pub fn num_cpus() -> io::Result<usize> {
    let rc = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc as usize)
    }
}
