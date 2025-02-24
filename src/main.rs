mod codec;
mod io_util;
mod objects;
#[macro_use]
mod proto;
mod state;

use std::{io, path::Path};

use io_util::{WlMsgReader, WlMsgWriter};
use state::WlMitmState;
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

    let mut upstream_read = WlMsgReader::new(upstream_read);
    let mut downstream_read = WlMsgReader::new(downstream_read);

    let mut upstream_write = WlMsgWriter::new(upstream_write);
    let mut downstream_write = WlMsgWriter::new(downstream_write);

    let mut state = WlMitmState::new();

    loop {
        tokio::select! {
            s2c_msg = upstream_read.read() => {
                match s2c_msg? {
                    codec::DecoderOutcome::Decoded(wl_raw_msg) => {
                        println!("s2c, obj_id = {}, opcode = {}", wl_raw_msg.obj_id, wl_raw_msg.opcode);

                        if state.on_s2c_event(&wl_raw_msg) {
                            downstream_write.write(wl_raw_msg).await?;
                        }
                    },
                    codec::DecoderOutcome::Incomplete => continue,
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            },
            c2s_msg = downstream_read.read() => {
                match c2s_msg? {
                    codec::DecoderOutcome::Decoded(wl_raw_msg) => {
                        println!("c2s, obj_id = {}, opcode = {}", wl_raw_msg.obj_id, wl_raw_msg.opcode);

                        if state.on_c2s_request(&wl_raw_msg) {
                            upstream_write.write(wl_raw_msg).await?;
                        }
                    },
                    codec::DecoderOutcome::Incomplete => continue,
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            }
        }
    }
}
