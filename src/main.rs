#![feature(async_await)]

use std::ffi::OsString;
use std::io;

use failure::{bail, format_err, Error};
use nix::sys::socket::SockAddr;

pub mod client;
pub mod fork;
pub mod lxcseccomp;
pub mod nsfd;
pub mod pidfd;
pub mod seccomp;
pub mod socket;
pub mod sys_mknod;
pub mod tools;

use socket::SeqPacketListener;

pub enum SyscallStatus {
    Ok(i64),
    Err(i32),
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {}", err);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let socket_path = std::env::args_os()
        .skip(1)
        .next()
        .ok_or_else(|| format_err!("missing parameter: socket path to listen on"))?;

    match std::fs::remove_file(&socket_path) {
        Ok(_) => (),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => (), // Ok
        Err(e) => bail!("failed to remove previous socket: {}", e),
    }

    tokio::run(async_run(socket_path));

    Ok(())
}

async fn async_run(socket_path: OsString) {
    if let Err(err) = async_run_do(socket_path).await {
        eprintln!("error accepting clients, bailing out: {}", err);
    }
}

async fn async_run_do(socket_path: OsString) -> Result<(), Error> {
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
