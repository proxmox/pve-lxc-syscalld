#![feature(async_await)]

use std::io;

use failure::{bail, format_err, Error};
use nix::sys::socket::SockAddr;

pub mod apparmor;
pub mod capability;
pub mod client;
pub mod fork;
pub mod lxcseccomp;
pub mod nsfd;
pub mod pidfd;
pub mod seccomp;
pub mod socket;
pub mod sys_mknod;
pub mod sys_quotactl;
pub mod syscall;
pub mod tools;

use socket::SeqPacketListener;

#[tokio::main]
async fn main() {
    if let Err(err) = do_main().await {
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

    let mut listener = SeqPacketListener::bind(&address)
        .map_err(|e| format_err!("failed to create listening socket: {}", e))?;
    loop {
        let client = listener.accept().await?;
        let client = client::Client::new(client);
        tokio::spawn(client.main());
    }
}
