use std::os::fd::OwnedFd;

use byteorder::{ByteOrder, NativeEndian};
use bytes::{Bytes, BytesMut};

pub struct WlRawMsg {
    // 4 bytes
    pub obj_id: u32,
    // 2 bytes
    pub len: u16,
    // 2 bytes
    pub opcode: u16,
    // len bytes -- containing the header
    msg_buf: Bytes,
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
            msg_buf: msg_buf.freeze(),
            fds: new_fds.into_boxed_slice(),
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.msg_buf[8..]
    }

    pub fn into_parts(self) -> (Bytes, Box<[OwnedFd]>) {
        (self.msg_buf, self.fds)
    }
}

pub enum DecoderOutcome {
    Decoded(WlRawMsg),
    Incomplete,
    Eof,
}

pub struct WlDecoder {
    buf: BytesMut,
    fds: Vec<OwnedFd>,
}

impl WlDecoder {
    pub fn new() -> WlDecoder {
        WlDecoder {
            buf: BytesMut::new(),
            fds: Vec::new(),
        }
    }

    pub fn decode_buf(&mut self) -> Option<DecoderOutcome> {
        if self.buf.len() == 0 {
            return None;
        }

        match WlRawMsg::try_decode(&mut self.buf, &mut self.fds) {
            Some(res) => Some(DecoderOutcome::Decoded(res)),
            None => Some(DecoderOutcome::Incomplete),
        }
    }

    pub fn decode_after_read(&mut self, buf: &[u8], fds: &mut Vec<OwnedFd>) -> DecoderOutcome {
        self.buf.extend_from_slice(&buf);
        self.fds.append(fds);

        match WlRawMsg::try_decode(&mut self.buf, &mut self.fds) {
            Some(res) => DecoderOutcome::Decoded(res),
            None => {
                if buf.len() == 0 {
                    DecoderOutcome::Eof
                } else {
                    DecoderOutcome::Incomplete
                }
            }
        }
    }
}
