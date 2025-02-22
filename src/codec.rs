use sendfd::{RecvWithFd, SendWithFd};
use std::io::Result;
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;

pub trait ReadFd {
    fn read_fd(&self, buf: &mut [u8], fd_buf: &mut [RawFd]) -> Result<(usize, usize)>;
}

pub trait WriteFd {
    fn write_fd(&self, buf: &[u8], fd_buf: &[RawFd]) -> Result<usize>;
}

impl ReadFd for UnixStream {
    fn read_fd(&self, buf: &mut [u8], fd_buf: &mut [RawFd]) -> Result<(usize, usize)> {
        self.recv_with_fd(buf, fd_buf)
    }
}

impl WriteFd for UnixStream {
    fn write_fd(&self, buf: &[u8], fd_buf: &[RawFd]) -> Result<usize> {
        self.send_with_fd(buf, fd_buf)
    }
}

pub struct WlRawMsg<'a> {
    pub sender: u32,
    pub len: u16,
    pub opcode: u16,
    pub payload: &'a [u8],
}

pub trait WlDecoder {}

pub struct WlDecoderImpl<T: ReadFd> {
    inner: T,
}

pub struct WlEncoder<T: WlDecoder> {
    inner: T,
}
