use std::io::Write;
use std::sync::mpsc::TryRecvError;

use mio::net::TcpStream;

use crate::http::{HttpError, HttpResponse};

pub enum State {
    Partial(Writer),
    Done(Box<HttpResponse>),
}

pub struct Writer {
    http_resp: Box<HttpResponse>,
    buffer: Vec<u8>,
    header_size: usize,
    bytes_written: usize,
}

impl Writer {
    pub fn new(http_resp: Box<HttpResponse>, mut buffer: Vec<u8>) -> Self {
        buffer.clear();
        http_resp.write_header(&mut buffer);

        Self {
            http_resp,
            header_size: buffer.len(),
            buffer,
            bytes_written: 0,
        }
    }

    pub fn write_tcp_stream(mut self, tcp_stream: &mut TcpStream) -> Result<State, HttpError> {
        if let Some(body) = self.http_resp.resp_body.take() {
            match body.try_recv() {
                Ok(body_part) => {
                    self.buffer.extend(&body_part);
                    self.http_resp.resp_body = Some(body);
                }
                Err(TryRecvError::Empty) => {
                    self.http_resp.resp_body = Some(body);
                }
                Err(TryRecvError::Disconnected) => {
                    // Sender has dropped - no more data
                }
            }
        }

        let bytes_written = tcp_stream
            .write(&self.buffer)
            .map_err(|e| HttpError::Io(("failed to write response to tcp stream", e)))?;

        let bytes_remaining = self.buffer.len() - bytes_written;
        for n in 0..bytes_remaining {
            self.buffer[n] = self.buffer[n + bytes_written];
        }

        self.buffer.truncate(bytes_remaining);
        self.bytes_written += bytes_written;

        if self.buffer.is_empty() && self.http_resp.resp_body.is_none() {
            self.http_resp.resp_content_length = Some(self.bytes_written - self.header_size);
            Ok(State::Done(self.http_resp))
        } else {
            Ok(State::Partial(self))
        }
    }
}
