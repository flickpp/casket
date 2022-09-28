use std::io::{self, Write};
use std::result;
use std::sync::mpsc::{Receiver, RecvError};

use mio::net::TcpStream;

use crate::http::{HttpResponse, ResponseEncoder};

pub enum WriteError {
    Io(io::Error),
}

pub enum WriteState {
    Continue(WritingStream),
    Done(bool),
}

pub type WriteResult = result::Result<WriteState, WriteError>;

pub struct WritingStream {
    buffer: Vec<u8>,
    body: Option<Receiver<Vec<u8>>>,
    keep_alive: bool,
}

impl WritingStream {
    pub fn new(http_resp: HttpResponse, body: Receiver<Vec<u8>>) -> Self {
        let mut resp_encoder = ResponseEncoder::new(http_resp.code, &http_resp.reason);

        for (name, value) in http_resp.headers {
            resp_encoder.write_header(&name, &value);
        }

        if http_resp.keep_alive {
            resp_encoder.write_header("Connection", "Keep-Alive");
        } else {
            resp_encoder.write_header("Connection", "Close");
        }

        resp_encoder.write_header("X-TraceId", &http_resp.context.trace_id);
        resp_encoder.write_header("Server", "Casket");

        Self {
            buffer: resp_encoder.into_buffer(),
            body: Some(body),
            keep_alive: http_resp.keep_alive,
        }
    }

    pub fn write_tcp_stream(mut self, tcp_stream: &mut TcpStream) -> WriteResult {
        if let Some(body) = self.body.take() {
            match body.recv() {
                Ok(body_part) => {
                    self.buffer.extend(&body_part);
                    self.body = Some(body);
                }
                Err(RecvError) => {
                    // Sender has dropped the sender
                    // There is no more body
                }
            }
        }

        let bytes_written = tcp_stream.write(&self.buffer).map_err(WriteError::Io)?;

        let bytes_remaining = self.buffer.len() - bytes_written;
        for n in 0..bytes_remaining {
            self.buffer[n] = self.buffer[n + bytes_written];
        }
        self.buffer.truncate(bytes_remaining);

        if self.buffer.is_empty() && self.body.is_none() {
            Ok(WriteState::Done(self.keep_alive))
        } else {
            Ok(WriteState::Continue(self))
        }
    }
}
