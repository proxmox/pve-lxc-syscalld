#![feature(async_await)]

use std::io;

use failure::{bail, format_err, Error};
use nix::sys::socket::SockAddr;

pub mod lxcseccomp;
pub mod seccomp;
pub mod socket;
pub mod tools;

use socket::{AsyncSeqPacketSocket, SeqPacketListener};

const SOCKET_DIR: &'static str = "/run/pve";
const SOCKET_PATH: &'static str = "/run/pve/lxc-syscalld.sock";

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {}", err);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let _ = std::fs::create_dir(SOCKET_DIR);

    match std::fs::remove_file(SOCKET_PATH) {
        Ok(_) => (),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => (), // Ok
        Err(e) => bail!("failed to remove previous socket: {}", e),
    }

    tokio::run(async_run());

    Ok(())
}

async fn async_run() {
    if let Err(err) = async_run_do().await {
        eprintln!("error accepting clients, bailing out: {}", err);
    }
}

async fn async_run_do() -> Result<(), Error> {
    let address = SockAddr::new_unix(SOCKET_PATH).expect("cannot create struct sockaddr_un?");

    let mut listener = SeqPacketListener::bind(&address)
        .map_err(|e| format_err!("failed to create listening socket: {}", e))?;
    loop {
        let client = listener.accept().await?;
        tokio::spawn(handle_client(client));
    }
}

async fn handle_client(client: AsyncSeqPacketSocket) {
    if let Err(err) = handle_client_do(client).await {
        eprintln!("error communicating with client, dropping connection: {}", err);
    }
}

async fn handle_client_do(mut client: AsyncSeqPacketSocket) -> Result<(), Error> {
    let mut msgbuf = lxcseccomp::ProxyMessageBuffer::new(64)
        .map_err(|e| format_err!("failed to allocate proxy message buffer: {}", e))?;

    loop {
        let (size, _fds) = client.recv_fds(unsafe { msgbuf.new_mut() }, 1).await?;
        if size == 0 {
            println!("client disconnected");
            break;
        }

        msgbuf.set_len(size)?;

        let req = msgbuf.request();
        println!("Received request for syscall {}", req.data.nr);

        let resp = msgbuf.response_mut();
        resp.val = 0;
        resp.error = -libc::ENOENT;

        client.sendmsg(msgbuf.as_buf_no_cookie()).await?;
    }

    Ok(())
}
