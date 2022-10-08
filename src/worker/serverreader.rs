use std::io::Read;

use mio::net::TcpStream;

use crate::http::{Context, HttpError, HttpRequest};

pub enum State {
    Partial(Reader),
    Complete(Box<HttpRequest>),
    StreamEOF,
}

enum InnerState {
    Begin((usize, Vec<u8>)),
    HaveHeader(Box<PartialHttpReq>),
}

pub struct Reader {
    state: InnerState,
}

impl Reader {
    pub fn new() -> Self {
        Self {
            state: InnerState::Begin((0, vec![0; 2048])),
        }
    }

    pub fn read_tcp_stream(self, tcp_stream: &mut TcpStream) -> Result<State, HttpError> {
        match self.state {
            InnerState::Begin((buf_len, buf)) => read_header(buf_len, buf, tcp_stream),
            InnerState::HaveHeader(mut partial_http_req) => {
                partial_http_req.read_tcp_stream(tcp_stream)?;

                if partial_http_req.is_done() {
                    Ok(State::Complete(Box::new((*partial_http_req).into())))
                } else {
                    Ok(State::Partial(Reader {
                        state: InnerState::HaveHeader(partial_http_req),
                    }))
                }
            }
        }
    }
}

fn read_header(
    mut buf_len: usize,
    mut buf: Vec<u8>,
    tcp_stream: &mut TcpStream,
) -> Result<State, HttpError> {
    if buf.len() - buf_len < 1024 {
        buf.resize(buf.len() * 2, 0);
    }

    let bytes_read = tcp_stream
        .read(&mut buf[buf_len..])
        .map_err(|e| HttpError::Io(("failed to read tcp stream for server request", e)))?;

    if bytes_read == 0 {
        return Ok(State::StreamEOF);
    }

    buf_len += bytes_read;

    let mut headers = [httparse::EMPTY_HEADER; 24];
    let mut request = httparse::Request::new(&mut headers);

    match request.parse(&buf[..buf_len]) {
        Err(e) => Err(HttpError::HeaderParse(e)),

        Ok(httparse::Status::Partial) => Ok(State::Partial(Reader {
            state: InnerState::Begin((buf_len, buf)),
        })),

        Ok(httparse::Status::Complete(header_size)) => {
            let mut partial_http_req = PartialHttpReq::new(request)?;
            buf.truncate(buf_len);
            partial_http_req.take_body(buf, header_size)?;

            if partial_http_req.is_done() {
                Ok(State::Complete(Box::new(partial_http_req.into())))
            } else {
                Ok(State::Partial(Reader {
                    state: InnerState::HaveHeader(Box::new(partial_http_req)),
                }))
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

impl PartialHttpReq {
    fn new(request: httparse::Request<'_, '_>) -> Result<Self, HttpError> {
        let mut headers = vec![];

        let method = request
            .method
            .expect("request not parsed")
            .parse::<http_types::Method>()
            .map_err(|_| HttpError::BadValue("http request with unrecognised method"))?;

        let mut content_type: Option<String> = None;
        let mut host: Option<&str> = None;
        let mut keep_alive = true;
        let mut content_length = 0;
        let mut context: Option<Context> = None;

        for h in request.headers {
            let value = std::str::from_utf8(h.value)
                .map_err(|_| HttpError::BadValue("header value not utf8"))?;

            if h.name.eq_ignore_ascii_case("Content-Type") {
                content_type = Some(value.to_string());
            }

            if h.name.eq_ignore_ascii_case("Content-Length") {
                content_length = value
                    .parse()
                    .map_err(|_| HttpError::BadValue("Content-Length not uint"))?;
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

        let host = host.ok_or(HttpError::BadValue("http request missing host header"))?;

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

    fn take_body(&mut self, buffer: Vec<u8>, header_size: usize) -> Result<(), HttpError> {
        if buffer[header_size..].len() > self.content_length {
            // Too many bytes in buffer
            return Err(HttpError::BadValue("content-length too large"));
        }

        self.body.reserve(self.content_length);
        self.body.extend(&buffer[header_size..]);
        self.body.resize(self.content_length, 0);
        self.bytes_read = buffer.len() - header_size;

        Ok(())
    }

    fn read_tcp_stream(&mut self, tcp_stream: &mut TcpStream) -> Result<(), HttpError> {
        let bytes_read = tcp_stream
            .read(&mut self.body[self.bytes_read..])
            .map_err(|e| HttpError::Io(("failed to ready request body on tcp stream", e)))?;

        if bytes_read == 0 {
            return Err(HttpError::BadValue("stream EOF without complete body"));
        }

        self.bytes_read += bytes_read;

        Ok(())
    }

    fn is_done(&self) -> bool {
        self.bytes_read == self.content_length
    }
}

impl From<PartialHttpReq> for HttpRequest {
    fn from(req: PartialHttpReq) -> HttpRequest {
        HttpRequest {
            method: req.method,
            url: req.url,
            headers: req.headers,
            context: req.context,
            keep_alive: req.keep_alive,
            content_type: req.content_type,
            content_length: req.content_length,
            body: Some(req.body),
        }
    }
}

fn url(host: &str, path: &str) -> Result<http_types::Url, HttpError> {
    if path.starts_with("http://") || path.starts_with("https://") {
        http_types::Url::parse(path).map_err(|_| HttpError::BadValue("invalid http path"))
    } else if path.starts_with('/') {
        http_types::Url::parse(&format!("http://{}{}", host, path))
            .map_err(|_| HttpError::BadValue("invalid path in http header"))
    } else {
        Err(HttpError::BadValue("invalid path in http header"))
    }
}

fn parse_context(val: &str) -> Result<Context, &'static str> {
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
