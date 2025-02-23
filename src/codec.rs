use std::{
    io,
    os::fd::{FromRawFd, OwnedFd},
};

use byteorder::{ByteOrder, NativeEndian};
use bytes::BytesMut;
use sendfd::RecvWithFd;

pub struct WlRawMsg {
    // 4 bytes
    pub obj_id: u32,
    // 2 bytes
    pub len: u16,
    // 2 bytes
    pub opcode: u16,
    // len bytes -- containing the header
    msg_buf: BytesMut,
    pub fds: Box<[OwnedFd]>,
}

impl WlRawMsg {
    pub fn try_decode(buf: &mut BytesMut, fds: &mut Vec<OwnedFd>) -> Option<WlRawMsg> {
        let buf_len = buf.len();
        // Not even a complete message header
        if buf_len < 8 {
            return None;
        }

        let msg_len_and_opcode = NativeEndian::read_u32(&buf[4..8]);
        let msg_len = msg_len_and_opcode >> 16;
        // Not a complete message
        if buf_len < msg_len as usize {
            return None;
        }

        let opcode = msg_len_and_opcode & 0xFF;
        let obj_id = NativeEndian::read_u32(&buf[0..4]);
        let msg_buf = buf.split_to(msg_len as usize);

        let mut new_fds = Vec::with_capacity(fds.len());
        new_fds.append(fds);

        Some(WlRawMsg {
            obj_id,
            len: msg_len as u16,
            opcode: opcode as u16,
            msg_buf,
            fds: new_fds.into_boxed_slice(),
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.msg_buf[8..]
    }

    pub fn into_parts(self) -> (BytesMut, Box<[OwnedFd]>) {
        (self.msg_buf, self.fds)
    }
}

pub enum DecoderOutcome {
    Decoded(WlRawMsg),
    Incomplete,
    Eof,
}

pub struct WlDecoder<T: RecvWithFd> {
    pub inner: T,
    buf: BytesMut,
    fds: Vec<OwnedFd>,
}

impl<T: RecvWithFd> WlDecoder<T> {
    pub fn new(inner: T) -> WlDecoder<T> {
        WlDecoder {
            inner,
            buf: BytesMut::new(),
            fds: Vec::new(),
        }
    }

    pub fn try_read(&mut self) -> io::Result<DecoderOutcome> {
        // If we can decode something from what have buffered before, don't even try to read more
        if let Some(res) = WlRawMsg::try_decode(&mut self.buf, &mut self.fds) {
            return Ok(DecoderOutcome::Decoded(res));
        }

        let mut tmp_buf = [0u8; 128];
        let mut tmp_fds = [0i32; 128];
        let (len_buf, len_fds) = self.inner.recv_with_fd(&mut tmp_buf, &mut tmp_fds)?;

        self.buf.extend_from_slice(&tmp_buf[0..len_buf]);

        for fd in &tmp_fds[0..len_fds] {
            self.fds.push(unsafe { OwnedFd::from_raw_fd(*fd) });
        }

        match WlRawMsg::try_decode(&mut self.buf, &mut self.fds) {
            Some(res) => Ok(DecoderOutcome::Decoded(res)),
            None => {
                if len_buf == 0 {
                    if self.buf.len() > 0 {
                        Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "unexpected EOF",
                        ))
                    } else {
                        Ok(DecoderOutcome::Eof)
                    }
                } else {
                    Ok(DecoderOutcome::Incomplete)
                }
            }
        }
    }
}
