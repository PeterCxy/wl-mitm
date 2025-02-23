mod codec;

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use codec::WlDecoder;
use sendfd::{RecvWithFd, SendWithFd};

fn main() {
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

    while let Ok(conn) = listener.accept() {
        println!("Accepted new client {:?}", conn.1);
        let upstream_conn = UnixStream::connect(src.clone())
            .expect("Failed to connect to upstream Wayland display");
        let _upstream_conn = upstream_conn.try_clone().unwrap();
        let downstream_conn = conn.0;
        let _downstream_conn = downstream_conn.try_clone().unwrap();
        std::thread::spawn(move || {
            forward("server->client", upstream_conn, downstream_conn).ok();
        });
        std::thread::spawn(move || {
            forward("client->server", _downstream_conn, _upstream_conn).ok();
        });
    }
}

fn forward(
    direction: &'static str,
    ingress: impl RecvWithFd,
    egress: impl SendWithFd,
) -> io::Result<()> {
    let mut decoder = WlDecoder::new(ingress);

    loop {
        match decoder.try_read()? {
            codec::DecoderOutcome::Decoded(wl_raw_msg) => {
                println!(
                    "{direction} obj_id: {}, opcode {}",
                    wl_raw_msg.obj_id, wl_raw_msg.opcode
                );
                wl_raw_msg.write_into(&egress)?;
            }
            codec::DecoderOutcome::Incomplete => continue,
            codec::DecoderOutcome::Eof => break,
        }
    }

    println!("Stream disconnected");
    Ok(())
}
