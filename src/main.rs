use std::os::unix::net::{UnixListener, UnixStream};
use std::os::fd::RawFd;
use std::path::Path;
use sendfd::{RecvWithFd, SendWithFd};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        println!("Usage: {} <wl_display> <wl_display_proxied>", args[0]);
        return;
    }

    let xdg_rt = std::env::var("XDG_RUNTIME_DIR")
        .expect("XDG_RUNTIME_DIR not set");

    let src = format!("{}/{}", xdg_rt, args[1]);
    let proxied = format!("{}/{}", xdg_rt, args[2]);

    if src == proxied {
        println!("downstream and upstream sockets should not be the same");
        return;
    }

    if Path::exists(proxied.as_ref()) {
        std::fs::remove_file(&proxied).expect("Cannot unlink existing socket");
    }

    let listener = UnixListener::bind(proxied)
        .expect("Failed to bind to target socket");

    while let Ok(conn) = listener.accept() {
        println!("Accepted new client {:?}", conn.1);
        let upstream_conn = UnixStream::connect(src.clone())
            .expect("Failed to connect to upstream Wayland display");
        let _upstream_conn = upstream_conn.try_clone().unwrap();
        let downstream_conn = conn.0;
        let _downstream_conn = downstream_conn.try_clone().unwrap();
        std::thread::spawn(move || {
            forward(upstream_conn, downstream_conn);
        });
        std::thread::spawn(move || {
            forward(_downstream_conn, _upstream_conn);
        });
    }
}

fn forward(conn1: UnixStream, conn2: UnixStream) {
    let mut buf = [0u8; 512];
    let mut fdbuf = [RawFd::default(); 512];

    while let Ok((len_data, len_fd)) = conn1.recv_with_fd(&mut buf, &mut fdbuf) {
        println!("Received data {} fd {}", len_data, len_fd);
        if len_data == 0 && len_fd == 0 {
            break;
        }
        if let Err(_) = conn2.send_with_fd(&buf[0..len_data], &fdbuf[0..len_fd]) {
            break;
        }
    }

    println!("Stream disconnected");
}
