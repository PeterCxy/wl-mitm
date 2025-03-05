mod codec;
mod io_util;
mod objects;
#[macro_use]
mod proto;
mod config;
mod state;

use std::{io, path::Path, sync::Arc};

use config::Config;
use io_util::{WlMsgReader, WlMsgWriter};
use proto::{WL_DISPLAY_OBJECT_ID, WlConstructableMessage, WlDisplayErrorEvent};
use state::{WlMitmOutcome, WlMitmState, WlMitmVerdict};
use tokio::net::{UnixListener, UnixStream};
use tracing::{Instrument, Level, debug, error, info, span};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut conf_file = "config.toml";

    let args: Vec<_> = std::env::args().collect();
    if args.len() >= 2 {
        conf_file = &args[2];
    }

    let conf_str = tokio::fs::read_to_string(conf_file)
        .await
        .expect("Can't read config file");
    let config: Arc<Config> =
        Arc::new(toml::from_str(&conf_str).expect("Can't decode config file"));

    let src = config.socket.upstream_socket_path();
    let proxied = config.socket.listen_socket_path();

    if src == proxied {
        error!("downstream and upstream sockets should not be the same");
        return;
    }

    if proxied.exists() {
        tokio::fs::remove_file(&proxied)
            .await
            .expect("Cannot unlink existing socket");
    }

    let listener = UnixListener::bind(&proxied).expect("Failed to bind to target socket");

    info!(path = ?proxied, "Listening on socket");

    let mut conn_id = 0;
    while let Ok((conn, addr)) = listener.accept().await {
        info!(conn_id = conn_id, "Accepted new client {:?}", addr);
        let span = span!(Level::INFO, "conn", conn_id = conn_id);
        let _config = config.clone();
        let _src = src.clone();
        tokio::spawn(
            async move {
                if let Err(e) = handle_conn(_config, _src, conn).await {
                    error!(error = ?e, "Failure handling connection");
                }
            }
            .instrument(span),
        );
        conn_id += 1;
    }
}

#[tracing::instrument(skip_all)]
pub async fn handle_conn(
    config: Arc<Config>,
    src_path: impl AsRef<Path>,
    mut downstream_conn: UnixStream,
) -> io::Result<()> {
    let mut upstream_conn = UnixStream::connect(src_path).await?;

    let (upstream_read, upstream_write) = upstream_conn.split();
    let (downstream_read, downstream_write) = downstream_conn.split();

    let mut upstream_read = WlMsgReader::new(upstream_read);
    let mut downstream_read = WlMsgReader::new(downstream_read);

    let mut upstream_write = WlMsgWriter::new(upstream_write);
    let mut downstream_write = WlMsgWriter::new(downstream_write);

    let mut state = WlMitmState::new(config);

    loop {
        tokio::select! {
            s2c_msg = upstream_read.read() => {
                match s2c_msg? {
                    codec::DecoderOutcome::Decoded(mut wl_raw_msg) => {
                        debug!(obj_id = wl_raw_msg.obj_id, opcode = wl_raw_msg.opcode, num_fds = wl_raw_msg.fds.len(), "s2c event");

                        let WlMitmOutcome(num_consumed_fds, verdict) = state.on_s2c_event(&wl_raw_msg).await;
                        upstream_read.return_unused_fds(&mut wl_raw_msg, num_consumed_fds);

                        match verdict {
                            WlMitmVerdict::Allowed => {
                                downstream_write.queue_write(wl_raw_msg);
                            },
                            WlMitmVerdict::Terminate => break Err(io::Error::new(io::ErrorKind::ConnectionAborted, "aborting connection")),
                            _ => {}
                        }
                    },
                    codec::DecoderOutcome::Incomplete => continue,
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            },
            c2s_msg = downstream_read.read() => {
                match c2s_msg? {
                    codec::DecoderOutcome::Decoded(mut wl_raw_msg) => {
                        debug!(obj_id = wl_raw_msg.obj_id, opcode = wl_raw_msg.opcode, num_fds = wl_raw_msg.fds.len(), "c2s request");

                        let WlMitmOutcome(num_consumed_fds, verdict) = state.on_c2s_request(&wl_raw_msg).await;
                        downstream_read.return_unused_fds(&mut wl_raw_msg, num_consumed_fds);

                        match verdict {
                            WlMitmVerdict::Allowed => {
                                upstream_write.queue_write(wl_raw_msg);
                            },
                            WlMitmVerdict::Rejected(error_code) => {
                                downstream_write.queue_write(
                                    WlDisplayErrorEvent::new(WL_DISPLAY_OBJECT_ID, wl_raw_msg.obj_id, error_code, "Rejected by wl-mitm").build()
                                );
                            },
                            WlMitmVerdict::Terminate => break Err(io::Error::new(io::ErrorKind::ConnectionAborted, "aborting connection")),
                            _ => {}
                        }
                    },
                    codec::DecoderOutcome::Incomplete => continue,
                    codec::DecoderOutcome::Eof => break Ok(()),
                }
            }
            // Try to write of we have any queued up. These don't do anything if no message is queued.
            res = upstream_write.dequeue_write() => res?,
            res = downstream_write.dequeue_write() => res?,
        }
    }
}
