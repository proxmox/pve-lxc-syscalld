use std::future::Future;
use std::io;

use failure::{bail, format_err, Error};
use nix::sys::socket::SockAddr;

#[macro_use]
mod macros;

pub mod apparmor;
pub mod capability;
pub mod client;
pub mod epoll;
pub mod error;
pub mod executor;
pub mod fork;
pub mod lxcseccomp;
pub mod nsfd;
pub mod process;
pub mod reactor;
pub mod rw_traits;
pub mod seccomp;
pub mod sys_mknod;
pub mod sys_quotactl;
pub mod syscall;
pub mod tools;

use io_uring::socket::SeqPacketListener;

static mut EXECUTOR: *mut executor::ThreadPool = std::ptr::null_mut();

pub fn executor() -> &'static executor::ThreadPool {
    unsafe { &*EXECUTOR }
}

pub fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    executor().spawn_ok(fut)
}

fn main() {
    let mut executor = executor::ThreadPool::new().expect("spawning worker threadpool");
    unsafe {
        EXECUTOR = &mut executor;
    }
    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);

    if let Err(err) = executor.run(do_main()) {
        eprintln!("error: {}", err);
        std::process::exit(1);
    }
}

async fn do_main() -> Result<(), Error> {
    let socket_path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| format_err!("missing parameter: socket path to listen on"))?;

    match std::fs::remove_file(&socket_path) {
        Ok(_) => (),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => (), // Ok
        Err(e) => bail!("failed to remove previous socket: {}", e),
    }

    let address =
        SockAddr::new_unix(socket_path.as_os_str()).expect("cannot create struct sockaddr_un?");

    let mut listener = SeqPacketListener::bind_default(&address)
        .map_err(|e| format_err!("failed to create listening socket: {}", e))?;
    loop {
        let client = listener.accept().await?;
        let client = client::Client::new(client);
        spawn(client.main());
    }
}
