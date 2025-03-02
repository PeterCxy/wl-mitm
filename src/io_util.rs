use std::{
    future::poll_fn,
    io,
    ops::Deref,
    os::fd::{FromRawFd, OwnedFd},
    task::{Context, Poll},
};

use bytes::Bytes;
use sendfd::{RecvWithFd, SendWithFd};
use tokio::net::unix::{ReadHalf, WriteHalf};

use crate::codec::{DecoderOutcome, WlDecoder, WlRawMsg};

pub struct WlMsgReader<'a> {
    ingress: ReadHalf<'a>,
    decoder: WlDecoder,
}

impl<'a> WlMsgReader<'a> {
    pub fn new(ingress: ReadHalf<'a>) -> Self {
        WlMsgReader {
            ingress,
            decoder: WlDecoder::new(),
        }
    }

    pub fn return_unused_fds(&mut self, msg: &mut WlRawMsg, num_consumed: usize) {
        self.decoder.return_unused_fds(msg, num_consumed);
    }

    pub async fn read(&mut self) -> io::Result<DecoderOutcome> {
        if let Some(DecoderOutcome::Decoded(msg)) = self.decoder.decode_buf() {
            return Ok(DecoderOutcome::Decoded(msg));
        }

        loop {
            self.ingress.readable().await?;

            let mut tmp_buf = [0u8; 128];
            let mut tmp_fds = [0i32; 128];

            let (read_bytes, read_fds) = match self.ingress.recv_with_fd(&mut tmp_buf, &mut tmp_fds)
            {
                Ok(res) => res,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            };

            let mut fd_vec: Vec<OwnedFd> = Vec::with_capacity(read_fds);
            for fd in &tmp_fds[0..read_fds] {
                fd_vec.push(unsafe { OwnedFd::from_raw_fd(*fd) });
            }

            return Ok(self
                .decoder
                .decode_after_read(&tmp_buf[0..read_bytes], fd_vec));
        }
    }
}

pub struct WlMsgWriter<'a> {
    egress: WriteHalf<'a>,
    write_queue: Vec<WlRawMsg>,
    cur_write_buf: Option<Bytes>,
    cur_write_buf_pos: usize,
    cur_write_fds: Option<Box<[OwnedFd]>>,
}

impl<'a> WlMsgWriter<'a> {
    pub fn new(egress: WriteHalf<'a>) -> Self {
        WlMsgWriter {
            egress,
            write_queue: Vec::new(),
            cur_write_buf: None,
            cur_write_buf_pos: 0,
            cur_write_fds: None,
        }
    }

    /// Can we possibly write anything?
    fn can_write(&self) -> bool {
        self.cur_write_buf.is_some() || !self.write_queue.is_empty()
    }

    /// Try to write __something__ into the underlying stream.
    /// This does not care about registering interests, so it may return ready with a WOULDBLOCK
    fn try_poll_write(&mut self) -> Poll<io::Result<()>> {
        // If we don't have a partially written buffer, try remove one from the write queue
        if self.cur_write_buf.is_none() && !self.write_queue.is_empty() {
            // Don't use pop(), wl messages need to be in order!!
            let (buf, fds) = self.write_queue.remove(0).into_parts();

            self.cur_write_buf = Some(buf);
            self.cur_write_buf_pos = 0;
            self.cur_write_fds = Some(fds);
        }

        if let Some(buf) = self.cur_write_buf.take() {
            let send_res = if let Some(fds) = self.cur_write_fds.take() {
                self.egress
                    .send_with_fd(&buf[self.cur_write_buf_pos..], unsafe {
                        std::mem::transmute(fds.deref())
                    })
            } else {
                self.egress
                    .send_with_fd(&buf[self.cur_write_buf_pos..], &[])
            };

            if let Ok(written) = send_res {
                // Partial send :(
                // At least fds are always guaranteed to be sent in full
                if self.cur_write_buf_pos + written < buf.len() {
                    self.cur_write_buf = Some(buf);
                    self.cur_write_buf_pos += written;
                }
            }

            // Caller is supposed to handle WOULDBLOCK
            Poll::Ready(send_res.map(|_| ()))
        } else {
            Poll::Pending
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // If we can't write anything, return pending immediately
        if !self.can_write() {
            return Poll::Pending;
        }

        while self.egress.as_ref().poll_write_ready(cx).is_ready() {
            match self.try_poll_write() {
                Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Poll::Ready(res) => return Poll::Ready(res),
                Poll::Pending => return Poll::Pending,
            }
        }

        Poll::Pending
    }

    /// Queue a message up for writing, but doesn't do anything right away.
    pub fn queue_write(&mut self, msg: WlRawMsg) {
        self.write_queue.push(msg);
    }

    /// Try to make progress by flushing some of the queued up messages into the stream.
    /// When this resolves, note that we might have only partially written. In that
    /// case the buffer is saved internally in this structure.
    ///
    /// The returned future will block forever (never resolve) if there is no
    /// message to be written. This behavior makes it play nicely with select!{}
    pub async fn dequeue_write(&mut self) -> io::Result<()> {
        poll_fn(|cx| self.poll_write(cx)).await
    }
}
