use std::result;
use std::time;

use mio::{net::TcpStream, Token};

use crate::http::{HttpError, HttpRequest, HttpResponse};

use super::serverreader;
use super::serverwriter;

const HTTP_408_RESPONSE: &[u8] = include_bytes!("http408");
const HTTP_503_RESPONSE: &[u8] = include_bytes!("http503");
const HTTP_504_RESPONSE: &[u8] = include_bytes!("http504");

pub struct CasketResponse {
    pub code: u16,
    pub response: Vec<u8>,
    pub reason: &'static str,
    pub bytes_sent: usize,
}

pub enum Action {
    NewServerRequest((Token, TcpStream)),
    ServerContinueRead((Token, serverreader::Reader, TcpStream)),
    ServerReadDone((Token, Box<HttpRequest>, TcpStream)),
    ServerStreamEOF((Token, TcpStream)),
    ServerNewResponse((Token, Box<HttpResponse>)),
    ServerContinueWrite((Token, serverwriter::Writer, TcpStream)),
    ServerDoneWrite((Token, Box<HttpResponse>, TcpStream)),

    ServerCasketResponseNew((Token, TcpStream, CasketResponse)),
    ServerCasketResponseContinue((Token, TcpStream, CasketResponse)),
    ServerCasketResponseDone((Token, TcpStream, CasketResponse)),

    ServerPythonCodeTimeoutNew((Token, time::SystemTime)),
}

pub fn new_408_timeout(tk: Token, tcp_stream: TcpStream) -> Action {
    Action::ServerCasketResponseNew((
        tk,
        tcp_stream,
        CasketResponse {
            code: 408,
            response: HTTP_408_RESPONSE.to_vec(),
            reason: "request read timeout",
            bytes_sent: 0,
        },
    ))
}

pub fn new_503_service_busy(tk: Token, tcp_stream: TcpStream) -> Action {
    Action::ServerCasketResponseNew((
        tk,
        tcp_stream,
        CasketResponse {
            code: 503,
            response: HTTP_503_RESPONSE.to_vec(),
            reason: "service busy",
            bytes_sent: 0,
        },
    ))
}

pub fn new_504_gateway_timeout(tk: Token, tcp_stream: TcpStream) -> Action {
    Action::ServerCasketResponseNew((
        tk,
        tcp_stream,
        CasketResponse {
            code: 504,
            response: HTTP_504_RESPONSE.to_vec(),
            reason: "gateway timeout",
            bytes_sent: 0,
        },
    ))
}

pub type WorkerResult<T> = result::Result<T, Error>;
pub type ActionResult = WorkerResult<Action>;

#[derive(Clone, Copy)]
pub enum ErrorSource {
    Server,
}

pub struct Error {
    pub source: ErrorSource,
    pub error: HttpError,
    pub token: Token,
    pub tcp_stream: TcpStream,
}
