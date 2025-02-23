use std::{
    io,
    ops::Deref,
    os::fd::{FromRawFd, OwnedFd},
};

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
                .decode_after_read(&tmp_buf[0..read_bytes], &mut fd_vec));
        }
    }
}

pub struct WlMsgWriter<'a> {
    egress: WriteHalf<'a>,
}

impl<'a> WlMsgWriter<'a> {
    pub fn new(egress: WriteHalf<'a>) -> Self {
        WlMsgWriter { egress }
    }

    pub async fn write(&mut self, msg: WlRawMsg) -> io::Result<()> {
        let (buf, fds) = msg.into_parts();

        let mut written = 0;

        while written < buf.len() {
            self.egress.writable().await?;

            let res = if written == 0 {
                self.egress
                    .send_with_fd(&buf, unsafe { std::mem::transmute(fds.deref()) })
            } else {
                self.egress.send_with_fd(&buf[written..], &[])
            };

            match res {
                Ok(new_written) => written += new_written,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}
