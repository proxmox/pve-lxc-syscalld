#![feature(async_await)]

use std::ffi::OsString;
use std::io;
use std::sync::Arc;

use failure::{bail, format_err, Error};
use nix::sys::socket::SockAddr;

pub mod lxcseccomp;
pub mod seccomp;
pub mod socket;
pub mod tools;

use socket::{AsyncSeqPacketSocket, SeqPacketListener};

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
        tokio::spawn(handle_client(Arc::new(client)));
    }
}

async fn handle_client(client: Arc<AsyncSeqPacketSocket>) {
    if let Err(err) = handle_client_do(client).await {
        eprintln!(
            "error communicating with client, dropping connection: {}",
            err
        );
    }
}

async fn handle_client_do(client: Arc<AsyncSeqPacketSocket>) -> Result<(), Error> {
    let mut msgbuf = lxcseccomp::ProxyMessageBuffer::new(64)
        .map_err(|e| format_err!("failed to allocate proxy message buffer: {}", e))?;

    loop {
        let (size, _fds) = {
            let mut iovec = msgbuf.io_vec_mut();
            client.recv_fds_vectored(&mut iovec, 1).await?
        };

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

        let iovec = msgbuf.io_vec_no_cookie();
        client.sendmsg_vectored(&iovec).await?;
    }

    Ok(())
}
