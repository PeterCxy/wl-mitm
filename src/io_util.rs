use std::{
    future::poll_fn,
    io,
    ops::Deref,
    os::fd::OwnedFd,
    task::{Context, Poll},
};

use bytes::BytesMut;
use sendfd::SendWithFd;
use tokio::net::unix::{ReadHalf, WriteHalf};

use crate::codec::{DecoderOutcome, WlDecoder, WlRawMsg};

pub struct WlMsgReader<'a> {
    ingress: WlDecoder<ReadHalf<'a>>,
}

impl<'a> WlMsgReader<'a> {
    pub fn new(ingress: ReadHalf<'a>) -> Self {
        let ingress = WlDecoder::new(ingress);

        WlMsgReader {
            ingress,
        }
    }

    fn poll_io(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<DecoderOutcome>> {
        while self.ingress.inner.as_ref().poll_read_ready(cx)?.is_ready() {
            match self.ingress.try_read() {
                Ok(outcome) => return Poll::Ready(Ok(outcome)),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }

        Poll::Pending
    }

    pub async fn io_next(&mut self) -> io::Result<DecoderOutcome> {
        poll_fn(|cx| self.poll_io(cx)).await
    }
}

pub struct WlMsgWriter<'a> {
    egress: WriteHalf<'a>,
    egress_msg_buf: Vec<WlRawMsg>,
    egress_pending_bytes: BytesMut,
    egress_pending_fds: Option<Box<[OwnedFd]>>,
}

impl<'a> WlMsgWriter<'a> {
    pub fn new(egress: WriteHalf<'a>) -> Self {
        WlMsgWriter {
            egress,
            egress_msg_buf: Vec::new(),
            egress_pending_bytes: BytesMut::new(),
            egress_pending_fds: None
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.egress_pending_bytes.is_empty() {
            if let Some(msg) = self.egress_msg_buf.pop() {
                let (bytes, fds) = msg.into_parts();
                self.egress_pending_bytes = bytes;
                self.egress_pending_fds = Some(fds);
            }
        }

        if !self.egress_pending_bytes.is_empty() {
            while self.egress.as_ref().poll_write_ready(cx)?.is_ready() {
                let send_res = if let Some(fds) = self.egress_pending_fds.as_ref() {
                    self.egress
                        .send_with_fd(&self.egress_pending_bytes, unsafe {
                            std::mem::transmute(fds.deref())
                        })
                } else {
                    self.egress.send_with_fd(&self.egress_pending_bytes, &[])
                };

                match send_res {
                    Ok(written) => {
                        self.egress_pending_fds = None;
                        _ = self.egress_pending_bytes.split_to(written);

                        if self.egress_pending_bytes.is_empty() {
                            return Poll::Ready(Ok(()));
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        }

        Poll::Pending
    }

    pub async fn queue_msg_write(&mut self, msg: WlRawMsg) -> io::Result<()> {
        self.egress_msg_buf.push(msg);

        poll_fn(|cx| {
            _ = self.poll_write(cx)?;
            Poll::Ready(Ok(()))
        }).await
    }

    pub async fn do_write(&mut self) -> io::Result<()> {
        poll_fn(|cx| {
            self.poll_write(cx)
        }).await
    }
}
