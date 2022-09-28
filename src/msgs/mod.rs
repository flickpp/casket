use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::unix::prelude::RawFd;

use fd_queue::{mio::UnixStream, DequeueFd, EnqueueFd};
use mio::Token;

pub struct ServerMsgBuffer {
    read_buffer: Vec<u8>,
    read_buf_len: usize,

    stream_tks: VecDeque<(Token, RawFd)>,
    stream_close_tks: VecDeque<(Token, RawFd)>,

    to_send: VecDeque<(Request, RawFd)>,
    write_buffer: Vec<u8>,
}

impl ServerMsgBuffer {
    pub fn new() -> Self {
        Self {
            read_buffer: vec![0; 2048],
            read_buf_len: 0,

            stream_tks: VecDeque::new(),
            stream_close_tks: VecDeque::new(),

            to_send: VecDeque::new(),
            write_buffer: vec![],
        }
    }

    pub fn read_unix_stream(&mut self, stream: &mut UnixStream) -> io::Result<()> {
        if self.read_buffer.len() - self.read_buf_len < 512 {
            self.read_buffer.resize(self.read_buffer.len() * 2, 0);
        }

        let buf = &mut self.read_buffer[self.read_buf_len..];
        self.read_buf_len += stream.read(buf)?;

        let mut bytes_read = 0;
        let mut buf = &self.read_buffer[..self.read_buf_len];

        while buf.len() > 4 {
            let size = (u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])) as usize;

            if buf.len() < (size + 4) {
                break;
            }

            let msg: Response =
                bincode::deserialize(&buf[4..(size + 4)]).expect("couldn't deserialize response");

            if msg.keep_alive && msg.error.is_none() {
                self.stream_tks.push_back((Token(msg.token), msg.fd));
            } else {
                self.stream_close_tks.push_back((Token(msg.token), msg.fd));
            }

            bytes_read += size + 4;
            buf = &buf[(size + 4)..];
        }

        let bytes_remaining = self.read_buf_len - bytes_read;
        for n in 0..bytes_remaining {
            self.read_buffer[n] = self.read_buffer[n + bytes_read];
        }
        self.read_buf_len = bytes_remaining;

        Ok(())
    }

    pub fn next_stream_tk(&mut self) -> Option<Token> {
        self.stream_tks.pop_front().map(|(tk, _)| tk)
    }

    pub fn next_stream_close_tk(&mut self) -> Option<Token> {
        self.stream_close_tks.pop_front().map(|(tk, _)| tk)
    }

    pub fn write_unix_stream(&mut self, stream: &mut UnixStream) -> io::Result<()> {
        while let Some((msg, fd)) = self.to_send.pop_front() {
            if stream.enqueue(&fd).is_err() {
                self.to_send.push_front((msg, fd));
                break;
            }

            let msg = bincode::serialize(&msg).expect("couldn't serialize msg");
            self.write_buffer.push(msg.len() as u8);
            self.write_buffer.extend(msg);
        }

        let bytes_written = stream.write(&self.write_buffer[..])?;
        let bytes_remaining = self.write_buffer.len() - bytes_written;

        for n in 0..bytes_remaining {
            self.write_buffer[n] = self.write_buffer[n + bytes_written];
        }
        self.write_buffer.truncate(bytes_remaining);

        Ok(())
    }

    pub fn has_data_to_send(&self) -> bool {
        !self.to_send.is_empty() || !self.write_buffer.is_empty()
    }

    pub fn req_tcp_stream_fd(&mut self, tk: Token, fd: RawFd) {
        let msg = Request { token: tk.0, fd };
        self.to_send.push_back((msg, fd));
    }
}

pub struct WorkerMsgBuffer {
    read_buffer: Vec<u8>,
    read_buf_len: usize,
    write_buffer: Vec<u8>,

    server_fds: HashMap<Token, RawFd>,
    stream_fds: VecDeque<RawFd>,
    stream_msgs: VecDeque<Request>,
}

impl WorkerMsgBuffer {
    pub fn new() -> Self {
        Self {
            read_buffer: vec![0; 2048],
            read_buf_len: 0,
            write_buffer: vec![],

            server_fds: HashMap::new(),
            stream_fds: VecDeque::new(),
            stream_msgs: VecDeque::new(),
        }
    }

    pub fn read_unix_stream(&mut self, stream: &mut UnixStream) -> io::Result<()> {
        let buf = &mut self.read_buffer[self.read_buf_len..];
        self.read_buf_len += stream.read(buf)?;

        let mut bytes_read = 0;
        let mut buf = &self.read_buffer[..self.read_buf_len];

        // Take our msgs
        while !buf.is_empty() {
            let size = buf[0] as usize;

            if buf.len() < (size + 1) {
                break;
            }

            let msg: Request =
                bincode::deserialize(&buf[1..(size + 1)]).expect("couldn't deserialize request");

            self.stream_msgs.push_back(msg);

            bytes_read += size + 1;
            buf = &buf[(size + 1)..];
        }

        let bytes_remaining = self.read_buf_len - bytes_read;
        for n in 0..bytes_remaining {
            self.read_buffer[n] = self.read_buffer[n + bytes_read];
        }
        self.read_buf_len = bytes_remaining;

        // Take our fds
        while let Some(fd) = stream.dequeue() {
            self.stream_fds.push_back(fd);
        }

        Ok(())
    }

    pub fn next_stream_fd(&mut self) -> Option<(Token, RawFd)> {
        let fd = match self.stream_fds.pop_front() {
            Some(fd) => fd,
            None => return None,
        };

        let msg = match self.stream_msgs.pop_front() {
            Some(msg) => msg,
            None => {
                self.stream_fds.push_front(fd);
                return None;
            }
        };

        // Save the server fd
        self.server_fds.insert(Token(msg.token), msg.fd);

        Some((Token(msg.token), fd))
    }

    pub fn has_data_to_send(&self) -> bool {
        !self.write_buffer.is_empty()
    }

    pub fn write_unix_stream(&mut self, stream: &mut UnixStream) -> io::Result<()> {
        let bytes_written = stream.write(&self.write_buffer[..])?;
        let bytes_remaining = self.write_buffer.len() - bytes_written;

        for n in 0..bytes_remaining {
            self.write_buffer[n] = self.write_buffer[n + bytes_written];
        }
        self.write_buffer.truncate(bytes_remaining);

        Ok(())
    }

    pub fn resp_io_error(&mut self, tk: Token, err: io::Error) {
        let resp = Response {
            token: tk.0,
            fd: self
                .server_fds
                .remove(&tk)
                .expect("couldn't find server fd"),
            keep_alive: false,
            error: Some(format!("{}-{}", "i/o error with stream", err)),
        };

        let msg = bincode::serialize(&resp).expect("couldn't serialize response");
        self.write_buffer.extend((msg.len() as u32).to_be_bytes());
        self.write_buffer.extend(msg);
    }

    pub fn resp_bad_client(&mut self, tk: Token) {
        let resp = Response {
            token: tk.0,
            fd: self
                .server_fds
                .remove(&tk)
                .expect("couldn't find server fd"),
            keep_alive: false,
            error: Some("badly formed client request".to_string()),
        };

        let msg = bincode::serialize(&resp).expect("couldn't serialize response");
        self.write_buffer.extend((msg.len() as u32).to_be_bytes());
        self.write_buffer.extend(msg);
    }

    pub fn resp_stream_reg_error(&mut self, tk: Token, err: io::Error) {
        let resp = Response {
            token: tk.0,
            fd: self
                .server_fds
                .remove(&tk)
                .expect("couldn't find server fd"),
            keep_alive: false,
            error: Some(format!("{}-{}", "couldn't register stream with mio", err)),
        };

        let msg = bincode::serialize(&resp).expect("couldn't serialize response");
        self.write_buffer.extend((msg.len() as u32).to_be_bytes());
        self.write_buffer.extend(msg);
    }

    pub fn resp_stream_done_ok(&mut self, tk: Token, _: RawFd, keep_alive: bool) {
        let resp = Response {
            token: tk.0,
            fd: self
                .server_fds
                .remove(&tk)
                .expect("couldn't find server fd"),
            keep_alive,
            error: None,
        };

        let msg = bincode::serialize(&resp).expect("couldn't serialize response");
        self.write_buffer.extend((msg.len() as u32).to_be_bytes());
        self.write_buffer.extend(msg);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    token: usize,
    fd: RawFd,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    token: usize,
    fd: RawFd,
    keep_alive: bool,
    error: Option<String>,
}
