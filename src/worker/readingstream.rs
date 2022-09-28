use std::io::{self, Read};
use std::result;

use mio::net::TcpStream;

use crate::http::{Context, HttpRequest};

pub enum ReadError {
    Io(io::Error),
    Httparse(httparse::Error),
    BadValue(&'static str),
}

pub enum ReadState {
    Partial(ReadingStream),
    Complete(Box<HttpRequest>),
}

enum InnerState {
    Begin((usize, Vec<u8>)),
    HaveHeader(Box<PartialHttpReq>),
}

pub type ReadResult = result::Result<ReadState, ReadError>;

pub struct ReadingStream {
    state: InnerState,
}

impl ReadingStream {
    pub fn empty() -> Self {
        Self {
            state: InnerState::Begin((0, vec![0; 2048])),
        }
    }

    pub fn read_tcp_stream(mut self, tcp_stream: &mut TcpStream) -> ReadResult {
        match self.state {
            InnerState::Begin((mut buf_len, mut buffer)) => {
                if buffer.len() - buf_len < 1024 {
                    buffer.resize(buffer.len() + 4096, 0);
                }

                buf_len += tcp_stream
                    .read(&mut buffer[buf_len..])
                    .map_err(ReadError::Io)?;

                // Parse the header
                let mut headers = [httparse::EMPTY_HEADER; 16];
                let mut request = httparse::Request::new(&mut headers);

                match request.parse(&buffer[..buf_len]) {
                    Ok(httparse::Status::Partial) => Ok(ReadState::Partial(Self {
                        state: InnerState::Begin((buf_len, buffer)),
                    })),
                    Err(err) => Err(ReadError::Httparse(err)),

                    Ok(httparse::Status::Complete(header_size)) => {
                        let mut partial_http_req = PartialHttpReq::new(request)?;
                        buffer.truncate(buf_len);
                        partial_http_req.take_body(buffer, header_size);

                        if partial_http_req.is_done() {
                            Ok(ReadState::Complete(Box::new(partial_http_req.into())))
                        } else {
                            self.state = InnerState::HaveHeader(Box::new(partial_http_req));
                            Ok(ReadState::Partial(self))
                        }
                    }
                }
            }

            InnerState::HaveHeader(mut partial_http_req) => {
                partial_http_req.read_tcp_stream(tcp_stream)?;

                if partial_http_req.is_done() {
                    Ok(ReadState::Complete(Box::new((*partial_http_req).into())))
                } else {
                    self.state = InnerState::HaveHeader(partial_http_req);
                    Ok(ReadState::Partial(self))
                }
            }
        }
    }
}

struct PartialHttpReq {
    method: http_types::Method,
    headers: Vec<(String, String)>,
    url: http_types::Url,
    content_type: Option<String>,
    content_length: usize,
    keep_alive: bool,
    body: Vec<u8>,
    bytes_read: usize,
    context: Context,
}

impl From<PartialHttpReq> for HttpRequest {
    fn from(req: PartialHttpReq) -> HttpRequest {
        HttpRequest {
            method: req.method,
            context: req.context,
            headers: req.headers,
            url: req.url,
            content_type: req.content_type,
            content_length: Some(req.content_length),
            body: req.body,
            keep_alive: req.keep_alive,
        }
    }
}

impl PartialHttpReq {
    fn new(request: httparse::Request<'_, '_>) -> result::Result<Self, ReadError> {
        let mut headers = vec![];

        let method = request
            .method
            .expect("request not parsed")
            .parse::<http_types::Method>()
            .map_err(|_| ReadError::BadValue("http request with unrecognised method"))?;

        let mut content_type: Option<String> = None;
        let mut host: Option<&str> = None;
        let mut keep_alive = true;
        let mut content_length = 0;
        let mut context: Option<Context> = None;

        for h in request.headers {
            let value = std::str::from_utf8(h.value)
                .map_err(|_| ReadError::BadValue("header value not utf8"))?;

            if h.name.eq_ignore_ascii_case("Content-Type") {
                content_type = Some(value.to_string());
            }

            if h.name.eq_ignore_ascii_case("Content-Length") {
                content_length = value
                    .parse()
                    .map_err(|_| ReadError::BadValue("Content-Length not uint"))?;
            }

            if h.name.eq_ignore_ascii_case("Host") {
                host = Some(value);
            }

            if h.name.eq_ignore_ascii_case("Traceparent") {
                if let Ok(ctx) = parse_context(value) {
                    context = Some(ctx);
                }
            }

            if h.name.eq_ignore_ascii_case("Connection") && value.eq_ignore_ascii_case("Close") {
                keep_alive = false;
            }

            headers.push((h.name.to_string(), value.to_string()));
        }

        let host = host.ok_or(ReadError::BadValue("http request missing host header"))?;

        Ok(Self {
            method,
            headers,
            content_type,
            content_length,
            keep_alive,
            body: vec![],
            bytes_read: 0,
            context: context.unwrap_or_else(Context::new),
            url: url(host, request.path.expect("request not parsed"))?,
        })
    }

    fn take_body(&mut self, buffer: Vec<u8>, header_size: usize) {
        self.body.reserve(self.content_length);
        self.body.extend(&buffer[header_size..]);
        self.body.resize(self.content_length, 0);
        self.bytes_read = buffer.len() - header_size;
    }

    fn read_tcp_stream(&mut self, tcp_stream: &mut TcpStream) -> result::Result<(), ReadError> {
        self.bytes_read += tcp_stream
            .read(&mut self.body[self.bytes_read..])
            .map_err(ReadError::Io)?;

        Ok(())
    }

    fn is_done(&self) -> bool {
        self.bytes_read == self.content_length
    }
}

fn url(host: &str, path: &str) -> result::Result<http_types::Url, ReadError> {
    if path.starts_with("http://") || path.starts_with("https://") {
        http_types::Url::parse(path).map_err(|_| ReadError::BadValue("invalid http path"))
    } else if path.starts_with('/') {
        http_types::Url::parse(&format!("http://{}{}", host, path))
            .map_err(|_| ReadError::BadValue("invalid path in http header"))
    } else {
        Err(ReadError::BadValue("invalid path in http header"))
    }
}

fn parse_context(val: &str) -> result::Result<Context, &'static str> {
    let mut trace_id = "";
    let mut parent_id = "";

    for (n, v) in val.split('-').enumerate() {
        match n {
            0 => {
                if v != "00" {
                    return Err("unsupported traceparent version");
                }
            }
            1 => trace_id = v,
            2 => parent_id = v,
            3 => {
                // Ignore any extensions
            }
            _ => return Err("traceparent header should have four parts"),
        }
    }

    if trace_id.len() != 32 || parent_id.len() != 16 {
        return Err("badly formed traceparent header");
    }

    for c in trace_id.chars() {
        if !"0123456789abcdef".contains(c) {
            return Err("trace_id is not a hexstring");
        }
    }

    for c in parent_id.chars() {
        if !"0123456789abcdef".contains(c) {
            return Err("parent_id is not a hexstring");
        }
    }

    Ok(Context::from_vals(trace_id, parent_id))
}
