mod codec;
mod io_util;
mod objects;
#[macro_use]
mod proto;
mod config;
mod state;

use std::{io, ops::ControlFlow, path::Path, str::FromStr, sync::Arc};

use codec::DecoderOutcome;
use config::Config;
use io_util::{WlMsgReader, WlMsgWriter};
use proto::{WL_DISPLAY_OBJECT_ID, WlConstructableMessage, WlDisplayErrorEvent};
use state::{WlMitmOutcome, WlMitmState, WlMitmVerdict};
use tokio::net::{UnixListener, UnixStream};
use tracing::{Instrument, Level, error, info, level_filters::LevelFilter, span, warn};

#[tokio::main]
async fn main() {
    let mut conf_file = "config.toml";

    let args: Vec<_> = std::env::args().collect();
    if args.len() >= 2 {
        conf_file = &args[1];
    }

    let conf_str = tokio::fs::read_to_string(conf_file)
        .await
        .expect("Can't read config file");
    let config: Arc<Config> =
        Arc::new(toml::from_str(&conf_str).expect("Can't decode config file"));

    let mut tracing_builder = tracing_subscriber::fmt();

    if let Some(ref level) = config.logging.log_level {
        tracing_builder = tracing_builder
            .with_max_level(LevelFilter::from_str(level).expect("Invalid log level"));
    }

    tracing_builder.init();

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

macro_rules! control_flow {
    ($f:expr) => {
        match $f {
            ControlFlow::Break(res) => break res,
            ControlFlow::Continue(_) => continue,
        }
    };
}

struct ConnDuplex<'a> {
    config: Arc<Config>,
    upstream_read: WlMsgReader<'a>,
    upstream_write: WlMsgWriter<'a>,
    downstream_read: WlMsgReader<'a>,
    downstream_write: WlMsgWriter<'a>,
    state: WlMitmState,
}

impl<'a> ConnDuplex<'a> {
    pub fn new(
        config: Arc<Config>,
        state: WlMitmState,
        upstream_conn: &'a mut UnixStream,
        downstream_conn: &'a mut UnixStream,
    ) -> Self {
        let (upstream_read, upstream_write) = upstream_conn.split();
        let (downstream_read, downstream_write) = downstream_conn.split();

        let upstream_read = WlMsgReader::new(upstream_read);
        let downstream_read = WlMsgReader::new(downstream_read);

        let upstream_write = WlMsgWriter::new(upstream_write);
        let downstream_write = WlMsgWriter::new(downstream_write);

        Self {
            config,
            upstream_read,
            upstream_write,
            downstream_read,
            downstream_write,
            state,
        }
    }

    async fn handle_s2c_event(
        &mut self,
        decoded_raw: DecoderOutcome,
    ) -> io::Result<ControlFlow<()>> {
        match decoded_raw {
            codec::DecoderOutcome::Decoded(mut wl_raw_msg) => {
                let WlMitmOutcome(num_consumed_fds, mut verdict) =
                    self.state.on_s2c_event(&wl_raw_msg).await;
                self.upstream_read
                    .return_unused_fds(&mut wl_raw_msg, num_consumed_fds);

                if !verdict.is_allowed() && self.config.filter.dry_run {
                    warn!(
                        verdict = ?verdict,
                        "Last event would have been filtered! (see prior logs for reason)"
                    );
                    verdict = WlMitmVerdict::Allowed;
                }

                match verdict {
                    WlMitmVerdict::Allowed => {
                        self.downstream_write.queue_write(wl_raw_msg);
                    }
                    WlMitmVerdict::Terminate => {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "aborting connection",
                        ));
                    }
                    _ => {}
                };
            }
            codec::DecoderOutcome::Eof => return Ok(ControlFlow::Break(())),
            _ => {}
        }

        Ok(ControlFlow::Continue(()))
    }

    async fn handle_c2s_request(
        &mut self,
        decoded_raw: DecoderOutcome,
    ) -> io::Result<ControlFlow<()>> {
        match decoded_raw {
            codec::DecoderOutcome::Decoded(mut wl_raw_msg) => {
                let WlMitmOutcome(num_consumed_fds, mut verdict) =
                    self.state.on_c2s_request(&wl_raw_msg).await;
                self.downstream_read
                    .return_unused_fds(&mut wl_raw_msg, num_consumed_fds);

                if !verdict.is_allowed() && self.config.filter.dry_run {
                    warn!(
                        verdict = ?verdict,
                        "Last request would have been filtered! (see prior logs for reason)"
                    );
                    verdict = WlMitmVerdict::Allowed;
                }

                match verdict {
                    WlMitmVerdict::Allowed => {
                        self.upstream_write.queue_write(wl_raw_msg);
                    }
                    WlMitmVerdict::Rejected(error_code) => {
                        self.downstream_write.queue_write(
                            WlDisplayErrorEvent::new(
                                WL_DISPLAY_OBJECT_ID,
                                wl_raw_msg.obj_id,
                                error_code,
                                "Rejected by wl-mitm",
                            )
                            .build(),
                        );
                    }
                    WlMitmVerdict::Terminate => {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "aborting connection",
                        ));
                    }
                    _ => {}
                }
            }
            codec::DecoderOutcome::Eof => return Ok(ControlFlow::Break(())),
            _ => {}
        }

        Ok(ControlFlow::Continue(()))
    }

    #[tracing::instrument(skip_all)]
    pub async fn run_to_completion(mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                msg = self.upstream_read.read() => {
                    control_flow!(self.handle_s2c_event(msg?).await?);
                }
                msg = self.downstream_read.read() => {
                    control_flow!(self.handle_c2s_request(msg?).await?);
                }
                res = self.upstream_write.dequeue_write() => res?,
                res = self.downstream_write.dequeue_write() => res?,
            }
        }

        Ok(())
    }
}

pub async fn handle_conn(
    config: Arc<Config>,
    src_path: impl AsRef<Path>,
    mut downstream_conn: UnixStream,
) -> io::Result<()> {
    let mut upstream_conn = UnixStream::connect(src_path).await?;
    let state = WlMitmState::new(config.clone());

    let duplex = ConnDuplex::new(config, state, &mut upstream_conn, &mut downstream_conn);

    duplex.run_to_completion().await
}
