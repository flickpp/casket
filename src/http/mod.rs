use std::sync::mpsc::Receiver;

use random_fast_rng::{FastRng, Random};

pub struct HttpRequest {
    pub method: http_types::Method,
    pub url: http_types::Url,
    pub headers: Vec<(String, String)>,
    pub context: Context,
    pub keep_alive: bool,
    pub content_type: Option<String>,
    pub content_length: usize,
    pub body: Option<Vec<u8>>,
}

pub struct HttpResponseHeader {
    pub code: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

pub struct HttpResponse {
    pub method: http_types::Method,
    pub url: http_types::Url,
    pub code: u16,
    pub reason: String,
    pub context: Context,
    pub keep_alive: bool,

    // req
    pub req_headers: Vec<(String, String)>,
    pub req_content_length: usize,

    // resp
    pub resp_headers: Vec<(String, String)>,
    pub resp_content_length: Option<usize>,
    pub resp_body: Option<Receiver<Vec<u8>>>,
}

impl HttpRequest {
    pub fn into_http_response(
        self,
        header: HttpResponseHeader,
        body: Receiver<Vec<u8>>,
    ) -> HttpResponse {
        let mut resp_content_length: Option<usize> = None;

        for (name, value) in header.headers.iter() {
            if name.eq_ignore_ascii_case("Content-Lenth") {
                if let Ok(cl) = value.parse::<usize>() {
                    resp_content_length = Some(cl);
                }
            }
        }

        HttpResponse {
            method: self.method,
            url: self.url,
            code: header.code,
            reason: header.reason,
            context: self.context,
            keep_alive: self.keep_alive,
            req_headers: self.headers,
            req_content_length: self.content_length,
            resp_headers: header.headers,
            resp_content_length,
            resp_body: Some(body),
        }
    }
}

impl HttpResponse {
    pub fn write_header(&self, buf: &mut Vec<u8>) {
        buf.clear();

        // Status line
        buf.extend(b"HTTP/1.1 ");
        buf.extend(self.code.to_string().as_bytes());
        buf.extend(b" ");
        buf.extend(self.reason.as_bytes());
        buf.extend(b" ");
        buf.extend("\r\n".as_bytes());

        // Headers
        for (name, value) in self.resp_headers.iter() {
            buf.extend(name.as_bytes());
            buf.extend(b": ");
            buf.extend(value.as_bytes());
            buf.extend(b"\r\n");
        }

        // Context
        buf.extend("X-TraceId".as_bytes());
        buf.extend(b": ");
        buf.extend(self.context.trace_id.as_bytes());
        buf.extend(b"\r\n");

        // Keep-Alive
        buf.extend("Connection".as_bytes());
        buf.extend(b": ");
        if self.keep_alive {
            buf.extend("Keep-Alive".as_bytes());
        } else {
            buf.extend("Close".as_bytes());
        }
        buf.extend(b"\r\n");

        // Server
        buf.extend("Server".as_bytes());
        buf.extend(b": ");
        buf.extend("Casket".as_bytes());
        buf.extend(b"\r\n");

        // Done
        buf.extend(b"\r\n");
    }
}

#[derive(Clone)]
pub struct Context {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
}

impl Context {
    pub fn new() -> Self {
        let mut rng = FastRng::new();
        let trace_id: [u8; 16] = rng.gen();
        let span_id: [u8; 8] = rng.gen();

        Self {
            trace_id: hex::encode(trace_id),
            span_id: hex::encode(span_id),
            parent_id: None,
        }
    }

    pub fn from_vals(trace_id: &str, parent_id: &str) -> Self {
        let mut rng = FastRng::new();
        let span_id: [u8; 8] = rng.gen();

        Self {
            trace_id: trace_id.to_string(),
            parent_id: Some(parent_id.to_string()),
            span_id: hex::encode(span_id),
        }
    }

    pub fn parent_id_as_ref(&self) -> Option<&str> {
        self.parent_id.as_ref().map(|s| &s[..])
    }
}
