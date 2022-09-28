use std::collections::{HashMap, VecDeque};
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::prelude::RawFd;
use std::result;
use std::sync::mpsc::{channel, TryRecvError};
use std::sync::Arc;
use std::time;

use fd_queue::mio::UnixStream;
use mio::{net::TcpStream, Events, Interest, Poll, Registry, Token};

use crate::config::Config;
use crate::errors::{fatal_io_error, RuntimeError, RuntimeResult};
use crate::http::{Context, HttpRequest, HttpResponse};
use crate::msgs;
use crate::pythonexec;

mod readingstream;
use readingstream::{ReadError, ReadState, ReadingStream};
mod writingstream;
use writingstream::{WriteError, WriteState, WritingStream};

lazy_static! {
    static ref TOO_BUSY_RESP: HttpResponse = HttpResponse {
        code: 503,
        reason: "Service Busy".to_string(),
        headers: vec![],
        context: Context::new(),
        keep_alive: false,
    };
}

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

enum Action {
    None,
    NewReadStream((Token, TcpStream)),
    ContinueReadStream((Token, TcpStream, ReadingStream)),
    NewWriteStream(pythonexec::Response),
    ContinueWriteStream((Token, TcpStream, WritingStream)),
    DoneWriteStream((Token, TcpStream, bool)),
    NewProcessStream((Token, TcpStream, Box<HttpRequest>)),
    NewBusyStream(Box<HttpRequest>),
}

enum Error {
    PythonThreadsDied,
    Read((Token, TcpStream, ReadError)),
    Write((Token, TcpStream, WriteError)),
}

type Result = result::Result<Action, Error>;

pub fn run_worker(
    cfg: Arc<Config>,
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

    loop {
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

        if poll.poll(&mut events, timeout).is_err() {
            // Exit gracefully - this is caused by Ctrl-C
            return Ok(());
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
                // Write from TcpStream
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
                    handle_error(err, poll.registry(), &mut msg_buffer)?;
                }
            }
        }
    }
}

fn handle_error(
    err: Error,
    registry: &Registry,
    msg_buffer: &mut msgs::WorkerMsgBuffer,
) -> RuntimeResult {
    match err {
        Error::PythonThreadsDied => Err(RuntimeError::PythonThreadsDied),
        Error::Read((tk, mut tcp_stream, read_error)) => match read_error {
            ReadError::Io(err) => {
                registry.deregister(&mut tcp_stream).unwrap_or(());
                msg_buffer.resp_io_error(tk, err);
                Ok(())
            }
            ReadError::Httparse(_) => {
                registry.deregister(&mut tcp_stream).unwrap_or(());
                msg_buffer.resp_bad_client(tk);
                Ok(())
            }
            ReadError::BadValue(_) => {
                registry.deregister(&mut tcp_stream).unwrap_or(());
                msg_buffer.resp_bad_client(tk);
                Ok(())
            }
        },
        Error::Write((tk, mut tcp_stream, write_error)) => match write_error {
            WriteError::Io(err) => {
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
        Action::ContinueReadStream((tk, tcp_stream, reading_stream)) => {
            tcp_streams
                .reading_streams
                .insert(tk, (tcp_stream, reading_stream));
        }
        Action::NewWriteStream(resp) => {
            let tk = Token(resp.key);
            let mut tcp_stream = tcp_streams
                .processing_streams
                .remove(&tk)
                .expect("processing streams deque is empty");

            if let Err(e) = registry.register(&mut tcp_stream, tk, Interest::WRITABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams.writing_streams.insert(
                tk,
                (tcp_stream, WritingStream::new(resp.http_resp, resp.body)),
            );
        }
        Action::ContinueWriteStream((tk, tcp_stream, writing_stream)) => {
            tcp_streams
                .writing_streams
                .insert(tk, (tcp_stream, writing_stream));
        }
        Action::DoneWriteStream((tk, mut tcp_stream, keep_alive)) => {
            if let Err(e) = registry.deregister(&mut tcp_stream) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            msg_buffer.resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), keep_alive);
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

            let (_, bod_recv) = channel();
            let writing_stream = WritingStream::new(too_busy_resp(http_req), bod_recv);

            if let Err(e) = registry.register(&mut tcp_stream, tk, Interest::WRITABLE) {
                msg_buffer.resp_stream_reg_error(tk, e);
                return;
            }

            tcp_streams
                .writing_streams
                .insert(tk, (tcp_stream, writing_stream));
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
    }
}

fn handle_write_stream(tk: Token, mut stream: TcpStream, writing_stream: WritingStream) -> Result {
    match writing_stream.write_tcp_stream(&mut stream) {
        Err(err) => Err(Error::Write((tk, stream, err))),

        Ok(WriteState::Continue(writing_stream)) => {
            Ok(Action::ContinueWriteStream((tk, stream, writing_stream)))
        }

        Ok(WriteState::Done(keep_alive)) => Ok(Action::DoneWriteStream((tk, stream, keep_alive))),
    }
}

fn handle_try_recv_processing_stream(
    res: result::Result<pythonexec::Response, TryRecvError>,
) -> Result {
    match res {
        Ok(resp) => Ok(Action::NewWriteStream(resp)),
        Err(TryRecvError::Empty) => Ok(Action::None),
        Err(TryRecvError::Disconnected) => Err(Error::PythonThreadsDied),
    }
}

fn too_busy_resp(http_req: Box<HttpRequest>) -> HttpResponse {
    let mut resp = TOO_BUSY_RESP.clone();
    resp.context = http_req.context;
    resp
}
