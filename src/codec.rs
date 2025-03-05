use std::{collections::VecDeque, os::fd::OwnedFd};

use byteorder::{ByteOrder, NativeEndian};
use bytes::{BufMut, Bytes, BytesMut};
use tracing::debug;

#[allow(unused)]
pub struct WlRawMsg {
    // 4 bytes
    pub obj_id: u32,
    // 2 bytes
    pub len: u16,
    // 2 bytes
    pub opcode: u16,
    // len bytes -- containing the header
    msg_buf: Bytes,
    /// All fds we have seen up until decoding this message frame
    /// fds aren't guaranteed to be separated between messages; therefore, there
    /// is no way for us to tell that all fds here belong to the current message
    /// without actually loading the Wayland XML protocols.
    ///
    /// Instead, downstream parsers should return any unused fds back to the decoder
    /// with [WlDecoder::return_unused_fds].
    pub fds: Vec<OwnedFd>,
}

impl WlRawMsg {
    pub fn try_decode(buf: &mut BytesMut, fds: &mut VecDeque<OwnedFd>) -> Option<WlRawMsg> {
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
        while let Some(fd) = fds.pop_front() {
            new_fds.push(fd);
        }

        Some(WlRawMsg {
            obj_id,
            len: msg_len as u16,
            opcode: opcode as u16,
            msg_buf: msg_buf.freeze(),
            fds: new_fds,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.msg_buf[8..]
    }

    pub fn into_parts(self) -> (Bytes, Box<[OwnedFd]>) {
        (self.msg_buf, self.fds.into_boxed_slice())
    }

    pub fn build(
        obj_id: u32,
        opcode: u16,
        builder: impl FnOnce(&mut BytesMut, &mut Vec<OwnedFd>),
    ) -> WlRawMsg {
        let mut fds = Vec::new();
        let mut buf = BytesMut::new();
        buf.put_u32_ne(obj_id);
        // We don't yet know the length of this message, so put a 0 as placeholder
        buf.put_u32_ne(0);

        builder(&mut buf, &mut fds);

        let len_and_opcode = ((buf.len() as u32) << 16 as u32) | (opcode as u32);
        debug!(len_and_opcode = len_and_opcode, "message len and opcode");
        NativeEndian::write_u32(&mut buf[4..8], len_and_opcode);

        debug!(buf = ?buf, "constructed message");

        WlRawMsg {
            obj_id,
            len: buf.len() as u16,
            opcode,
            msg_buf: buf.freeze(),
            fds,
        }
    }
}

pub enum DecoderOutcome {
    Decoded(WlRawMsg),
    Incomplete,
    Eof,
}

pub struct WlDecoder {
    buf: BytesMut,
    fds: VecDeque<OwnedFd>,
}

impl WlDecoder {
    pub fn new() -> WlDecoder {
        WlDecoder {
            buf: BytesMut::new(),
            fds: VecDeque::new(),
        }
    }

    pub fn return_unused_fds(&mut self, msg: &mut WlRawMsg, num_consumed: usize) {
        let mut unused = msg.fds.split_off(num_consumed);

        // Add all unused vectors, in order, to the _front_ of our queue
        // This means that we take one item from the _back_ of the unused
        // chunk at a time and insert that to the _front_, to preserve order.
        while let Some(fd) = unused.pop() {
            self.fds.push_front(fd);
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

    pub fn decode_after_read(&mut self, buf: &[u8], fds: Vec<OwnedFd>) -> DecoderOutcome {
        self.buf.extend_from_slice(&buf);
        self.fds.extend(fds.into_iter());

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
