mod codec;
mod io_util;

use std::{io, path::Path};

use io_util::WlMsgIo;
use tokio::net::{UnixListener, UnixStream};

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        println!("Usage: {} <wl_display> <wl_display_proxied>", args[0]);
        return;
    }

    let xdg_rt = std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR not set");

    let src = format!("{}/{}", xdg_rt, args[1]);
    let proxied = format!("{}/{}", xdg_rt, args[2]);

    if src == proxied {
        println!("downstream and upstream sockets should not be the same");
        return;
    }

    if Path::exists(proxied.as_ref()) {
        std::fs::remove_file(&proxied).expect("Cannot unlink existing socket");
    }

    let listener = UnixListener::bind(proxied).expect("Failed to bind to target socket");

    while let Ok((conn, addr)) = listener.accept().await {
        println!("Accepted new client {:?}", addr);
        tokio::spawn(handle_conn(src.clone(), conn));
    }
}

pub async fn handle_conn(src_path: String, mut downstream_conn: UnixStream) -> io::Result<()> {
    let mut upstream_conn = UnixStream::connect(src_path).await?;

    let (upstream_read, upstream_write) = upstream_conn.split();
    let (downstream_read, downstream_write) = downstream_conn.split();

    let mut s2c = WlMsgIo::new(upstream_read, downstream_write);
    let mut c2s = WlMsgIo::new(downstream_read, upstream_write);

    loop {
        tokio::select! {
            s2c_msg = s2c.io_next() => {
                match s2c_msg? {
                    codec::DecoderOutcome::Decoded(wl_raw_msg) => {
                        println!("s2c, obj_id = {}, opcode = {}", wl_raw_msg.obj_id, wl_raw_msg.opcode);
                        s2c.queue_msg_write(wl_raw_msg);
                    },
                    codec::DecoderOutcome::Incomplete => {
                        println!("s2c, incomplete message");
                        continue
                    },
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            },
            c2s_msg = c2s.io_next() => {
                match c2s_msg? {
                    codec::DecoderOutcome::Decoded(wl_raw_msg) => {
                        println!("c2s, obj_id = {}, opcode = {}", wl_raw_msg.obj_id, wl_raw_msg.opcode);
                        c2s.queue_msg_write(wl_raw_msg);
                    },
                    codec::DecoderOutcome::Incomplete => continue,
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            }
        }
    }
}
