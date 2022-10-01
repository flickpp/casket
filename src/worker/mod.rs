use std::collections::{HashMap, VecDeque};
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::prelude::RawFd;
use std::result;
use std::sync::mpsc::TryRecvError;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time;

use fd_queue::mio::UnixStream;
use mio::{net::TcpStream, Events, Interest, Poll, Registry, Token};
use ndjsonlogger::info;

use crate::config::Config;
use crate::errors::{fatal_io_error, RuntimeError, RuntimeResult};
use crate::http::{HttpRequest, HttpResponse};
use crate::msgs;
use crate::pythonexec;

mod readingstream;
use readingstream::{ReadError, ReadState, ReadingStream};
mod writingstream;
use writingstream::{WriteError, WriteState, WritingStream};

#[derive(Default)]
struct TcpStreams {
    // Streams registered for reading
    reading_streams: HashMap<Token, (TcpStream, ReadingStream)>,

    // Streams, whose data is currently being processed in Python threads
    processing_streams: HashMap<Token, TcpStream>,

    // Streams on which we will return worker too busy
    too_busy_streams: VecDeque<(Token, TcpStream)>,

    // Streams registered for writing
    writing_streams: HashMap<Token, (TcpStream, WritingStream)>,
}

impl TcpStreams {
    fn is_empty(&self) -> bool {
        self.reading_streams.is_empty()
            && self.processing_streams.is_empty()
            && self.too_busy_streams.is_empty()
            && self.writing_streams.is_empty()
    }
}

enum Action {
    None,
    NewReadStream((Token, TcpStream)),
    ContinueReadStream((Token, TcpStream, ReadingStream)),
    NewWriteStream((usize, Box<HttpResponse>)),
    ContinueWriteStream((Token, TcpStream, WritingStream)),
    DoneWriteStream((Token, TcpStream, Box<HttpResponse>)),
    NewProcessStream((Token, TcpStream, Box<HttpRequest>)),
    NewBusyStream(Box<HttpRequest>),
    StreamEOF((Token, TcpStream)),
}

enum Error {
    PythonThreadsDied,
    Read((Token, TcpStream, ReadError)),
    Write((Token, TcpStream, WriteError)),
}

type Result = result::Result<Action, Error>;

pub fn run_worker(
    cfg: Arc<Config>,
    running: Arc<AtomicBool>,
    close_now: Arc<AtomicBool>,
    application: pythonexec::Application,
    mut stream: UnixStream,
) -> RuntimeResult {
    // Spawn Python threads
    let server = (cfg.hostname.clone(), cfg.port());
    let (mut req_send, resp_recv) = pythonexec::spawn(cfg.clone(), application, server);

    let mut poll = Poll::new().map_err(|e| fatal_io_error("couldn't create poll instance", e))?;
    let mut events = Events::with_capacity(64);

    // Register our unix stream for communicating with server
    const STREAM_TOKEN: Token = Token(0);
    let mut unix_stream_interest = Interest::READABLE;
    poll.registry()
        .register(&mut stream, STREAM_TOKEN, unix_stream_interest)
        .map_err(|e| fatal_io_error("couldn't register unix stream with mio in worker", e))?;

    let mut tcp_streams = TcpStreams::default();
    let mut msg_buffer = msgs::WorkerMsgBuffer::new();

    let mut results = Vec::with_capacity(64);
    let mut http_reqs: Vec<(Token, HttpRequest)> = Vec::with_capacity(16);
    let mut http_too_busy_reqs: Vec<HttpRequest> = Vec::with_capacity(16);
    let mut closing = false;
    let mut ctrlc_instant: Option<time::SystemTime> = None;

    loop {
        // Gracefully close after SIGINT
        if closing && tcp_streams.is_empty() && !msg_buffer.has_data_to_send() {
            break Ok(());
        }

        // Close due to CTRLC_WAIT_TIME expiring
        if let Some(instant) = ctrlc_instant {
            match instant.elapsed() {
                Err(_) => break Ok(()),
                Ok(duration) => {
                    if duration > cfg.ctrlc_wait_time {
                        break Ok(());
                    }
                }
            }
        }

        // Close now due to second SIGINT
        if close_now.load(Ordering::SeqCst) {
            break Ok(());
        }

        // If we have responses, make sure our unix stream is registered for writing
        if msg_buffer.has_data_to_send() && !unix_stream_interest.is_writable() {
            unix_stream_interest = Interest::READABLE | Interest::WRITABLE;
            poll.registry()
                .reregister(&mut stream, STREAM_TOKEN, unix_stream_interest)
                .map_err(|err| fatal_io_error("couldn't reregister unix stream in worker", err))?;
        }

        // If we don't have responses, make sure sure our unix stream is not registered for writing
        if !msg_buffer.has_data_to_send() && unix_stream_interest.is_writable() {
            unix_stream_interest = Interest::READABLE;
            poll.registry()
                .reregister(&mut stream, STREAM_TOKEN, unix_stream_interest)
                .map_err(|err| fatal_io_error("couldn't reregister unix stream in worker", err))?;
        }

        // Send parsed requests to python threads
        for (tk, req) in http_reqs.drain(..) {
            if req_send.send((tk.0, req)).is_err() {
                results.push(Err(Error::PythonThreadsDied));
            }
        }

        // Handle parsed requests which we're too busy too deal with
        for busy_req in http_too_busy_reqs.drain(..) {
            results.push(Ok(Action::NewBusyStream(Box::new(busy_req))));
        }

        let timeout = if !tcp_streams.processing_streams.is_empty() {
            // If we have processing streams (i.e reqs in python threads)
            // then don't block forever on poll. Furthermore call try_recv
            // on response queue.
            results.push(handle_try_recv_processing_stream(resp_recv.try_recv()));
            Some(time::Duration::from_millis(20))
        } else {
            None
        };

        let poll_res = poll.poll(&mut events, timeout);
        if poll_res.is_err() || !running.load(Ordering::SeqCst) {
            closing = true;
            ctrlc_instant = Some(time::SystemTime::now());
        }

        for ev in &events {
            // Unix Stream
            if ev.token() == STREAM_TOKEN {
                if ev.is_readable() {
                    // I/O errors on our unix stream are critical
                    msg_buffer
                        .read_unix_stream(&mut stream)
                        .map_err(|e| fatal_io_error("read error on unix stream in worker", e))?;

                    while let Some((token, fd)) = msg_buffer.next_stream_fd() {
                        results.push(handle_new_fd(token, fd));
                    }
                }

                if ev.is_writable() {
                    msg_buffer
                        .write_unix_stream(&mut stream)
                        .map_err(|e| fatal_io_error("write error on unix stream in worker", e))?;
                }

                // Next event
                continue;
            }

            // TcpStreams
            if ev.is_readable() {
                // Read from TcpStream
                let (tcp_stream, reading_stream) = tcp_streams
                    .reading_streams
                    .remove(&ev.token())
                    .expect("couldn't find reading stream");

                results.push(handle_read_stream(ev.token(), tcp_stream, reading_stream));
            }

            if ev.is_writable() {
                // Write to TcpStream
                let (tcp_stream, writing_stream) = tcp_streams
                    .writing_streams
                    .remove(&ev.token())
                    .expect("couldn't find reading stream");

                results.push(handle_write_stream(ev.token(), tcp_stream, writing_stream));
            }
        }

        for res in results.drain(..) {
            match res {
                Ok(action) => handle_action(
                    &cfg,
                    action,
                    poll.registry(),
                    &mut tcp_streams,
                    &mut http_reqs,
                    &mut http_too_busy_reqs,
                    &mut msg_buffer,
                ),
                Err(err) => {
                    handle_error(&cfg, err, poll.registry(), &mut msg_buffer)?;
                }
            }
        }
    }
}

fn handle_error(
    cfg: &Config,
    err: Error,
    registry: &Registry,
    msg_buffer: &mut msgs::WorkerMsgBuffer,
) -> RuntimeResult {
    match err {
        Error::PythonThreadsDied => Err(RuntimeError::PythonThreadsDied),
        Error::Read((tk, mut tcp_stream, read_error)) => {
            let error = match read_error {
                ReadError::Io(err) => {
                    registry.deregister(&mut tcp_stream).unwrap_or(());
                    msg_buffer.resp_io_error(tk, err);
                    "i/o error reading tcp stream"
                }
                ReadError::Httparse(_) => {
                    registry.deregister(&mut tcp_stream).unwrap_or(());
                    msg_buffer.resp_bad_client(tk);
                    "invalid http header"
                }
                ReadError::BadValue(err) => {
                    registry.deregister(&mut tcp_stream).unwrap_or(());
                    msg_buffer.resp_bad_client(tk);
                    err
                }
            };

            if cfg.log_response {
                info!("failed to read http request", { error });
            }

            Ok(())
        }
        Error::Write((tk, mut tcp_stream, write_error)) => match write_error {
            WriteError::Io((err, http_resp)) => {
                if cfg.log_response {
                    info!("failed to write http response to tcp stream", {
                        error                    = &format!("{}", err),
                        trace_id                 = &http_resp.context.trace_id,
                        span_id                  = &http_resp.context.span_id,
                        parent_id: Option<&str>  = http_resp.context.parent_id_as_ref()
                    });
                }
                registry.deregister(&mut tcp_stream).unwrap_or(());
                msg_buffer.resp_io_error(tk, err);
                Ok(())
            }
        },
    }
}

fn handle_action(
    cfg: &Config,
    action: Action,
    registry: &Registry,
    tcp_streams: &mut TcpStreams,
    http_reqs: &mut Vec<(Token, HttpRequest)>,
    http_too_busy_reqs: &mut Vec<HttpRequest>,
    msg_buffer: &mut msgs::WorkerMsgBuffer,
) {
    match action {
        Action::None => {}
        Action::NewReadStream((tk, mut tcp_stream)) => {
            if let Err(e) = registry.register(&mut tcp_stream, tk, Interest::READABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams
                .reading_streams
                .insert(tk, (tcp_stream, ReadingStream::empty()));
        }
        Action::ContinueReadStream((tk, mut tcp_stream, reading_stream)) => {
            // Read again
            if let Err(e) = registry.reregister(&mut tcp_stream, tk, Interest::READABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams
                .reading_streams
                .insert(tk, (tcp_stream, reading_stream));
        }
        Action::NewWriteStream((key, http_resp)) => {
            let tk = Token(key);
            let mut tcp_stream = tcp_streams
                .processing_streams
                .remove(&tk)
                .expect("processing streams deque is empty");

            if let Err(e) = registry.register(&mut tcp_stream, tk, Interest::WRITABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            let buffer = vec![0; 4096];

            tcp_streams
                .writing_streams
                .insert(tk, (tcp_stream, WritingStream::new(http_resp, buffer)));
        }
        Action::ContinueWriteStream((tk, mut tcp_stream, writing_stream)) => {
            // We need to register to write again
            if let Err(e) = registry.reregister(&mut tcp_stream, tk, Interest::WRITABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams
                .writing_streams
                .insert(tk, (tcp_stream, writing_stream));
        }
        Action::DoneWriteStream((tk, mut tcp_stream, http_resp)) => {
            if cfg.log_response {
                info!("request complete", {
                    "http.status_code": u16           = http_resp.code,
                    "http.method"                     = http_resp.method.as_ref(),
                    "http.url.path"                   = http_resp.url.path(),
                    "http.req_content_length" :usize  = http_resp.req_content_length,
                    "http.resp_content_length":usize  = http_resp.resp_content_length.unwrap_or(0),
                    trace_id                          = &http_resp.context.trace_id,
                    span_id                           = &http_resp.context.span_id,
                    parent_id: Option<&str>           = http_resp.context.parent_id_as_ref()
                });
            }

            if let Err(e) = registry.deregister(&mut tcp_stream) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            msg_buffer.resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), http_resp.keep_alive);
        }
        Action::NewProcessStream((tk, mut tcp_stream, http_req)) => {
            if let Err(e) = registry.deregister(&mut tcp_stream) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            if tcp_streams.processing_streams.len() < cfg.max_requests {
                tcp_streams.processing_streams.insert(tk, tcp_stream);
                http_reqs.push((tk, *http_req));
            } else {
                tcp_streams.too_busy_streams.push_back((tk, tcp_stream));
                http_too_busy_reqs.push(*http_req);
            }
        }
        Action::NewBusyStream(http_req) => {
            let (tk, mut tcp_stream) = tcp_streams
                .too_busy_streams
                .pop_front()
                .expect("too busy streams deque empty");

            let writing_stream = WritingStream::new(too_busy_resp(http_req), vec![0; 512]);

            if let Err(e) = registry.register(&mut tcp_stream, tk, Interest::WRITABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams
                .writing_streams
                .insert(tk, (tcp_stream, writing_stream));
        }
        Action::StreamEOF((tk, mut tcp_stream)) => {
            if let Err(e) = registry.deregister(&mut tcp_stream) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            msg_buffer.resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), false);
        }
    }
}

fn handle_new_fd(tk: Token, fd: RawFd) -> Result {
    let stream = unsafe { TcpStream::from_raw_fd(fd) };

    Ok(Action::NewReadStream((tk, stream)))
}

fn handle_read_stream(tk: Token, mut stream: TcpStream, reading_stream: ReadingStream) -> Result {
    match reading_stream.read_tcp_stream(&mut stream) {
        Err(err) => Err(Error::Read((tk, stream, err))),

        Ok(ReadState::Partial(reading_stream)) => {
            Ok(Action::ContinueReadStream((tk, stream, reading_stream)))
        }

        Ok(ReadState::Complete(http_req)) => Ok(Action::NewProcessStream((tk, stream, http_req))),

        Ok(ReadState::StreamEOF) => Ok(Action::StreamEOF((tk, stream))),
    }
}

fn handle_write_stream(tk: Token, mut stream: TcpStream, writing_stream: WritingStream) -> Result {
    match writing_stream.write_tcp_stream(&mut stream) {
        Err(err) => Err(Error::Write((tk, stream, err))),

        Ok(WriteState::Continue(writing_stream)) => {
            Ok(Action::ContinueWriteStream((tk, stream, writing_stream)))
        }

        Ok(WriteState::Done(http_resp)) => Ok(Action::DoneWriteStream((tk, stream, http_resp))),
    }
}

fn handle_try_recv_processing_stream(
    res: result::Result<(usize, HttpResponse), TryRecvError>,
) -> Result {
    match res {
        Ok((key, http_resp)) => Ok(Action::NewWriteStream((key, Box::new(http_resp)))),
        Err(TryRecvError::Empty) => Ok(Action::None),
        Err(TryRecvError::Disconnected) => Err(Error::PythonThreadsDied),
    }
}

fn too_busy_resp(http_req: Box<HttpRequest>) -> Box<HttpResponse> {
    Box::new(HttpResponse {
        method: http_req.method,
        url: http_req.url,
        code: 503,
        reason: String::from("Service Busy"),
        context: http_req.context,
        keep_alive: false,
        req_headers: http_req.headers,
        req_content_length: http_req.content_length,
        resp_headers: vec![],
        resp_content_length: Some(0),
        resp_body: None,
    })
}
