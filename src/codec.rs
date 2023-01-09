use std::io::{Read, Write, Result};
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use sendfd::{RecvWithFd, SendWithFd};

pub trait ReadFd: Read {
    fn read_fd(&self, buf: &mut [u8], fd_buf: &mut [RawFd]) -> Result<(usize, usize)>;
}

pub trait WriteFd: Write {
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
