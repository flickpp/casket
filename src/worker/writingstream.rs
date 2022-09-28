use std::io::{self, Write};
use std::result;
use std::sync::mpsc::RecvError;

use mio::net::TcpStream;

use crate::http::HttpResponse;

pub enum WriteError {
    Io((io::Error, Box<HttpResponse>)),
}

pub enum WriteState {
    Continue(WritingStream),
    Done(Box<HttpResponse>),
}

pub type WriteResult = result::Result<WriteState, WriteError>;

pub struct WritingStream {
    http_resp: Box<HttpResponse>,
    buffer: Vec<u8>,
    header_sz: usize,
    bytes_written: usize,
}

impl WritingStream {
    pub fn new(http_resp: Box<HttpResponse>, mut buffer: Vec<u8>) -> Self {
        http_resp.write_header(&mut buffer);

        Self {
            http_resp,
            header_sz: buffer.len(),
            buffer,
            bytes_written: 0,
        }
    }

    pub fn write_tcp_stream(mut self, tcp_stream: &mut TcpStream) -> WriteResult {
        if let Some(body) = self.http_resp.resp_body.take() {
            match body.recv() {
                Ok(body_part) => {
                    self.buffer.extend(&body_part);
                    self.http_resp.resp_body = Some(body);
                }
                Err(RecvError) => {
                    // Sender has dropped the sender
                    // There is no more body
                }
            }
        }

        let bytes_written_res = tcp_stream.write(&self.buffer);

        if let Ok(bytes_written) = bytes_written_res {
            self.bytes_written += bytes_written;

            let bytes_remaining = self.buffer.len() - bytes_written;
            for n in 0..bytes_remaining {
                self.buffer[n] = self.buffer[n + bytes_written];
            }
            self.buffer.truncate(bytes_remaining);
        }

        match bytes_written_res {
            Err(err) => Err(WriteError::Io((err, self.http_resp))),
            Ok(_) => {
                if self.buffer.is_empty() && self.http_resp.resp_body.is_none() {
                    self.http_resp.resp_content_length = Some(self.bytes_written - self.header_sz);
                    Ok(WriteState::Done(self.http_resp))
                } else {
                    Ok(WriteState::Continue(self))
                }
            }
        }
    }
}
