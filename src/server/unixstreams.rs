use std::io;
use std::os::unix::prelude::RawFd;

use fd_queue::mio::UnixStream as MioUnixStream;
use mio::{Interest, Registry, Token};

use crate::msgs;

#[derive(Clone, Copy)]
enum StreamInterest {
    Not,
    Read,
    Write,
    ReadWrite,
}

impl StreamInterest {
    fn from_rw(poll_read: bool, poll_write: bool) -> Self {
        use StreamInterest::*;

        match (poll_read, poll_write) {
            (false, false) => Not,
            (false, true) => Write,
            (true, false) => Read,
            (true, true) => ReadWrite,
        }
    }
}

pub struct UnixStream {
    token: Token,
    stream: MioUnixStream,
    msg_buffer: msgs::ServerMsgBuffer,
    interest: StreamInterest,
    num_reqs: usize,
}

impl UnixStream {
    pub fn new(token: Token, stream: MioUnixStream) -> Self {
        Self {
            token,
            stream,
            num_reqs: 0,
            interest: StreamInterest::Not,
            msg_buffer: msgs::ServerMsgBuffer::new(),
        }
    }

    pub fn read_stream(&mut self) -> io::Result<()> {
        self.msg_buffer.read_unix_stream(&mut self.stream)
    }

    pub fn write_stream(&mut self) -> io::Result<()> {
        self.msg_buffer.write_unix_stream(&mut self.stream)
    }

    fn reregister(&mut self, registry: &Registry) -> io::Result<()> {
        let poll_read = self.num_reqs > 0;
        let poll_write = self.msg_buffer.has_data_to_send();

        match self.interest {
            StreamInterest::Not => {
                if poll_read && poll_write {
                    let int = Interest::READABLE | Interest::WRITABLE;
                    registry.register(&mut self.stream, self.token, int)?;
                }

                if !poll_read && poll_write {
                    registry.register(&mut self.stream, self.token, Interest::WRITABLE)?;
                }

                if poll_read && !poll_write {
                    registry.register(&mut self.stream, self.token, Interest::READABLE)?;
                }
            }
            StreamInterest::Read => {
                if !poll_read && !poll_write {
                    registry.deregister(&mut self.stream)?;
                }

                if !poll_read && poll_write {
                    registry.reregister(&mut self.stream, self.token, Interest::WRITABLE)?;
                }

                if poll_read && poll_write {
                    let int = Interest::READABLE | Interest::WRITABLE;
                    registry.reregister(&mut self.stream, self.token, int)?;
                }
            }
            StreamInterest::Write => {
                if !poll_read && !poll_write {
                    registry.deregister(&mut self.stream)?;
                }

                if poll_read && !poll_write {
                    registry.reregister(&mut self.stream, self.token, Interest::READABLE)?;
                }

                if poll_read && poll_write {
                    let int = Interest::READABLE | Interest::WRITABLE;
                    registry.reregister(&mut self.stream, self.token, int)?;
                }
            }
            StreamInterest::ReadWrite => {
                if !poll_read && !poll_write {
                    registry.deregister(&mut self.stream)?;
                }

                if !poll_read && poll_write {
                    registry.reregister(&mut self.stream, self.token, Interest::WRITABLE)?;
                }

                if poll_read && !poll_write {
                    registry.reregister(&mut self.stream, self.token, Interest::READABLE)?;
                }
            }
        }

        self.interest = StreamInterest::from_rw(poll_read, poll_write);
        Ok(())
    }

    fn next_stream_tk(&mut self) -> Option<Token> {
        match self.msg_buffer.next_stream_tk() {
            Some(tk) => {
                self.num_reqs -= 1;
                Some(tk)
            }
            None => None,
        }
    }

    fn next_stream_close_tk(&mut self) -> Option<Token> {
        match self.msg_buffer.next_stream_close_tk() {
            Some(tk) => {
                self.num_reqs -= 1;
                Some(tk)
            }
            None => None,
        }
    }

    fn msg_send_tcp_stream(&mut self, tk: Token, fd: RawFd) {
        self.num_reqs += 1;
        self.msg_buffer.req_tcp_stream_fd(tk, fd);
    }
}

pub struct UnixStreams {
    streams: Vec<UnixStream>,
}

impl UnixStreams {
    pub fn new(streams: Vec<UnixStream>) -> Self {
        Self { streams }
    }

    pub fn get_mut(&mut self, tk: Token) -> Option<&'_ mut UnixStream> {
        for stream in self.streams.iter_mut() {
            if stream.token == tk {
                return Some(stream);
            }
        }

        None
    }

    pub fn next_stream_tks(&mut self) -> Vec<Token> {
        let mut tks = vec![];

        for stream in self.streams.iter_mut() {
            while let Some(tk) = stream.next_stream_tk() {
                tks.push(tk);
            }
        }

        tks
    }

    pub fn next_stream_close_tks(&mut self) -> Vec<Token> {
        let mut tks = vec![];

        for stream in self.streams.iter_mut() {
            while let Some(tk) = stream.next_stream_close_tk() {
                tks.push(tk);
            }
        }

        tks
    }

    pub fn msg_send_tcp_stream(&mut self, tk: Token, fd: RawFd) {
        let mut ind = 0;
        let mut num_reqs = usize::MAX;

        for (n, stream) in self.streams.iter().enumerate() {
            if stream.num_reqs < num_reqs {
                ind = n;
                num_reqs = stream.num_reqs;
            }
        }

        self.streams
            .get_mut(ind)
            .unwrap()
            .msg_send_tcp_stream(tk, fd);
    }

    pub fn reregister(&mut self, registry: &Registry) -> Vec<io::Error> {
        let mut errors = vec![];
        for stream in self.streams.iter_mut() {
            if let Err(e) = stream.reregister(registry) {
                errors.push(e);
            }
        }

        errors
    }
}
