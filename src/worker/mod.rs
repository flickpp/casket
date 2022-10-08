use std::collections::HashMap;
use std::os::unix::io::FromRawFd;
use std::os::unix::io::IntoRawFd;
use std::sync::Arc;
use std::time;

use fd_queue::mio::UnixStream;
use mio::{net::TcpStream, Token};
use ndjsonlogger::info;

use crate::config::Config;
use crate::errors::{fatal_io_error, RuntimeResult};
use crate::http::HttpError;
use crate::msgs;
use crate::pythonexec;

mod actions;
use actions::{
    new_408_timeout, new_503_service_busy, new_504_gateway_timeout, Action, ActionResult,
    CasketResponse, Error as ActionError, ErrorSource,
};
mod events;
use events::Event;
mod poller;
mod pythonthreads;
mod serverreader;
mod serverwriter;

const UNIX_STREAM_TOKEN: Token = Token(0);
const NO_TOKEN: Token = Token(1);
const POLL_TIME: time::Duration = time::Duration::from_millis(20);

struct Worker {
    msg_buf: msgs::WorkerMsgBuffer,
    poll: poller::Poller,
    python_threads: pythonthreads::PythonThreads,

    server_reading_streams: HashMap<Token, (TcpStream, serverreader::Reader)>,
    server_pending_streams: HashMap<Token, TcpStream>,
    server_writing_streams: HashMap<Token, (TcpStream, serverwriter::Writer)>,
    server_casket_responses: HashMap<Token, (TcpStream, CasketResponse)>,
}

pub fn run_worker(
    cfg: Arc<Config>,
    application: pythonexec::Application,
    mut unix_stream: UnixStream,
) -> RuntimeResult {
    let mut poll = poller::Poller::new()
        .map_err(|e| fatal_io_error("worker couldn't create poll instance", e))?;

    poll.register_read(&mut unix_stream, UNIX_STREAM_TOKEN, Event::UnixStreamRead)
        .map_err(|e| fatal_io_error("worker couldn't register unix stream for reading", e))?;

    let mut worker = Worker {
        msg_buf: msgs::WorkerMsgBuffer::new(),
        poll,
        python_threads: pythonthreads::PythonThreads::new(cfg.clone(), application),

        server_reading_streams: HashMap::new(),
        server_pending_streams: HashMap::new(),
        server_writing_streams: HashMap::new(),
        server_casket_responses: HashMap::new(),
    };

    let mut events_buf = Vec::with_capacity(64);
    let mut events_timeout_buf = Vec::with_capacity(64);
    let mut worker_results = Vec::with_capacity(64);
    let mut closing = false;

    loop {
        if closing
            && !worker.msg_buf.has_data_to_send()
            && worker.python_threads.num_pending_reqs() == 0
            && !worker.python_threads.has_queued_reqs()
            && worker.server_writing_streams.is_empty()
        {
            break Ok(());
        }

        while let Some((tk, fd)) = worker.msg_buf.next_stream_fd() {
            events_buf.push((tk, Event::NewStreamFd(fd)));
        }

        if worker.python_threads.num_pending_reqs() > 0 {
            events_buf.push((NO_TOKEN, Event::PollPythonResponses));
        }

        if worker.python_threads.has_queued_reqs() {
            events_buf.push((NO_TOKEN, Event::QueuedRequests));
        }

        let timeout = if events_buf.is_empty() {
            None
        } else {
            Some(POLL_TIME)
        };

        worker.poll.tick(&mut events_buf, timeout)?;

        for (tk, ev) in events_buf.drain(..) {
            match ev {
                Event::CtrlC => {
                    closing = true;
                }
                Event::UnixStreamRead => {
                    worker
                        .msg_buf
                        .read_unix_stream(&mut unix_stream)
                        .map_err(|e| fatal_io_error("worker couldn't read unix stream", e))?;
                }
                Event::UnixStreamWrite => {
                    worker
                        .msg_buf
                        .write_unix_stream(&mut unix_stream)
                        .map_err(|e| fatal_io_error("worker couldn't read unix stream", e))?;
                }
                Event::NewStreamFd(fd) => {
                    let tcp_stream = unsafe { TcpStream::from_raw_fd(fd) };

                    if worker.python_threads.num_pending_reqs() >= cfg.max_requests {
                        worker_results.push(Ok(new_503_service_busy(tk, tcp_stream)));
                    } else {
                        worker_results.push(Ok(Action::NewServerRequest((tk, tcp_stream))));
                    }
                }
                Event::ServerStreamRead => {
                    let (tcp_stream, reader) = worker
                        .server_reading_streams
                        .remove(&tk)
                        .expect("couldn't find reading stream");

                    worker_results.push(event_server_stream_read(tk, tcp_stream, reader));
                }
                Event::QueuedRequests => worker.python_threads.send_queued_requests()?,
                Event::PollPythonResponses => {
                    worker.python_threads.take_responses(&mut worker_results)?
                }
                Event::ServerStreamWrite => {
                    let (tcp_stream, writer) = worker
                        .server_writing_streams
                        .remove(&tk)
                        .expect("couldn't find writing stream");

                    worker_results.push(event_server_stream_write(tk, tcp_stream, writer));
                }
                Event::RequestReadTimeout => {
                    events_timeout_buf.push((tk, events::Timeout::RequestRead));
                }
                Event::CasketResponseWrite => {
                    let (tcp_stream, casket_resp) = worker
                        .server_casket_responses
                        .remove(&tk)
                        .expect("couldn't find request read timeout stream");

                    worker_results.push(event_casket_response_write(tk, tcp_stream, casket_resp));
                }
                Event::PythonCodeTimeout => {
                    events_timeout_buf.push((tk, events::Timeout::PythonCode));
                }
            }
        }

        // Action all non-timeout events
        for res in worker_results.drain(..) {
            match res {
                Ok(act) => handle_action(&cfg, &mut worker, act),
                Err(e) => handle_error(&mut worker, e),
            }
        }

        // Timeout events
        for (tk, ev) in events_timeout_buf.drain(..) {
            match ev {
                events::Timeout::RequestRead => {
                    if let Some((mut tcp_stream, _)) = worker.server_reading_streams.remove(&tk) {
                        worker.poll.deregister(&mut tcp_stream).map_err(|e| {
                            fatal_io_error("worker couldn't deregister stream poll", e)
                        })?;
                        worker_results.push(Ok(new_408_timeout(tk, tcp_stream)));
                    }
                }
                events::Timeout::PythonCode => {
                    if let Some(tcp_stream) = worker.server_pending_streams.remove(&tk) {
                        worker.python_threads.timeout_request(tk);
                        worker_results.push(Ok(new_504_gateway_timeout(tk, tcp_stream)));
                    }
                }
            }
        }

        // Timeout results
        for res in worker_results.drain(..) {
            match res {
                Ok(act) => handle_action(&cfg, &mut worker, act),
                Err(e) => handle_error(&mut worker, e),
            }
        }

        // Put UnixStream in R or RW mode
        if worker.msg_buf.has_data_to_send() {
            worker
                .poll
                .reregister_rw(
                    &mut unix_stream,
                    UNIX_STREAM_TOKEN,
                    Event::UnixStreamRead,
                    Event::UnixStreamWrite,
                )
                .map_err(|e| fatal_io_error("worker failed to reregister unix stream", e))?;
        } else {
            worker
                .poll
                .reregister_read(&mut unix_stream, UNIX_STREAM_TOKEN, Event::UnixStreamRead)
                .map_err(|e| fatal_io_error("worker failed to reregister unix stream", e))?;
        }
    }
}

fn handle_action(cfg: &Config, worker: &mut Worker, act: Action) {
    use Action::*;

    match act {
        NewServerRequest((tk, mut tcp_stream)) => {
            if let Err(e) = worker
                .poll
                .register_read(&mut tcp_stream, tk, Event::ServerStreamRead)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            let timeout = time::SystemTime::now() + cfg.request_read_timeout;

            worker
                .poll
                .timer_event(tk, timeout, Event::RequestReadTimeout);

            worker
                .server_reading_streams
                .insert(tk, (tcp_stream, serverreader::Reader::new()));
        }
        ServerContinueRead((tk, reader, mut tcp_stream)) => {
            if let Err(e) =
                worker
                    .poll
                    .reregister_read(&mut tcp_stream, tk, Event::ServerStreamRead)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .server_reading_streams
                .insert(tk, (tcp_stream, reader));
        }
        ServerReadDone((tk, http_req, mut tcp_stream)) => {
            if let Err(e) = worker.poll.deregister(&mut tcp_stream) {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker.python_threads.queue_http_req(tk, http_req);
            worker.server_pending_streams.insert(tk, tcp_stream);
        }
        ServerStreamEOF((tk, mut tcp_stream)) => {
            if let Err(e) = worker.poll.deregister(&mut tcp_stream) {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .msg_buf
                .resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), false);
        }
        ServerNewResponse((tk, http_resp)) => {
            let mut tcp_stream = worker
                .server_pending_streams
                .remove(&tk)
                .expect("worker couldn't find pending stream");

            if let Err(e) =
                worker
                    .poll
                    .register_write(&mut tcp_stream, tk, Event::ServerStreamWrite)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker.server_writing_streams.insert(
                tk,
                (
                    tcp_stream,
                    serverwriter::Writer::new(http_resp, vec![0; 2048]),
                ),
            );
        }
        ServerContinueWrite((tk, writer, mut tcp_stream)) => {
            if let Err(e) =
                worker
                    .poll
                    .reregister_write(&mut tcp_stream, tk, Event::ServerStreamWrite)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .server_writing_streams
                .insert(tk, (tcp_stream, writer));
        }
        ServerDoneWrite((tk, http_resp, mut tcp_stream)) => {
            info!("sent HTTP response", {
                "http.status_code": u16           = http_resp.code,
                "http.method"                     = http_resp.method.as_ref(),
                "http.url.path"                   = http_resp.url.path(),
                "http.req_content_length" :usize  = http_resp.req_content_length,
                "http.resp_content_length":usize  = http_resp.resp_content_length.unwrap_or(0),
                trace_id                          = &http_resp.context.trace_id,
                span_id                           = &http_resp.context.span_id,
                parent_id: Option<&str>           = http_resp.context.parent_id_as_ref()
            });

            if let Err(e) = worker.poll.deregister(&mut tcp_stream) {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .msg_buf
                .resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), http_resp.keep_alive);
        }

        ServerCasketResponseNew((tk, mut tcp_stream, casket_resp)) => {
            if let Err(e) =
                worker
                    .poll
                    .register_write(&mut tcp_stream, tk, Event::CasketResponseWrite)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .server_casket_responses
                .insert(tk, (tcp_stream, casket_resp));
        }

        ServerCasketResponseContinue((tk, mut tcp_stream, casket_resp)) => {
            if let Err(e) =
                worker
                    .poll
                    .reregister_write(&mut tcp_stream, tk, Event::CasketResponseWrite)
            {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            worker
                .server_casket_responses
                .insert(tk, (tcp_stream, casket_resp));
        }
        ServerCasketResponseDone((tk, mut tcp_stream, casket_resp)) => {
            if let Err(e) = worker.poll.deregister(&mut tcp_stream) {
                worker.msg_buf.resp_stream_reg_error(tk, e);
                return;
            }

            info!("casket sent error http response", {
                "http.status_code": u16 = casket_resp.code,
                "reason" = casket_resp.reason
            });

            worker
                .msg_buf
                .resp_stream_done_ok(tk, tcp_stream.into_raw_fd(), false);
        }

        ServerPythonCodeTimeoutNew((tk, st)) => {
            worker
                .poll
                .timer_event(tk, st + cfg.python_code_timeout, Event::PythonCodeTimeout);
        }
    }
}

fn handle_error(worker: &mut Worker, mut error: actions::Error) {
    // Logging
    match error.error {
        HttpError::Io((reason, ref err)) => {
            info!("i/o failed on tcp stream", {
                reason,
                error = &format!("{}", err)
            });
        }
        HttpError::HeaderParse(e) => {
            info!("failed to parse http header", { error = &format!("{}", e) });
        }
        HttpError::BadValue(error) => {
            info!("invalid http", { error });
        }
    }

    if let Err(e) = worker.poll.deregister(&mut error.tcp_stream) {
        worker.msg_buf.resp_stream_reg_error(error.token, e);
        return;
    }

    match error.source {
        ErrorSource::Server => match error.error {
            HttpError::Io((_, err)) => worker.msg_buf.resp_io_error(error.token, err),

            HttpError::HeaderParse(_) => {
                // TODO: Send Bad request

                worker.msg_buf.resp_bad_client(error.token);
            }
            HttpError::BadValue(_) => {
                // TODO: Send Bad Request

                worker.msg_buf.resp_bad_client(error.token);
            }
        },
    }
}

fn event_server_stream_read(
    tk: Token,
    mut tcp_stream: TcpStream,
    reader: serverreader::Reader,
) -> ActionResult {
    use serverreader::State::*;

    match reader.read_tcp_stream(&mut tcp_stream) {
        Err(error) => Err(ActionError {
            token: tk,
            error,
            source: ErrorSource::Server,
            tcp_stream,
        }),
        Ok(Partial(reader)) => Ok(Action::ServerContinueRead((tk, reader, tcp_stream))),
        Ok(Complete(http_req)) => Ok(Action::ServerReadDone((tk, http_req, tcp_stream))),
        Ok(StreamEOF) => Ok(Action::ServerStreamEOF((tk, tcp_stream))),
    }
}

fn event_server_stream_write(
    tk: Token,
    mut tcp_stream: TcpStream,
    writer: serverwriter::Writer,
) -> ActionResult {
    use serverwriter::State::*;

    match writer.write_tcp_stream(&mut tcp_stream) {
        Err(error) => Err(ActionError {
            token: tk,
            error,
            source: ErrorSource::Server,
            tcp_stream,
        }),
        Ok(Partial(writer)) => Ok(Action::ServerContinueWrite((tk, writer, tcp_stream))),
        Ok(Done(http_resp)) => Ok(Action::ServerDoneWrite((tk, http_resp, tcp_stream))),
    }
}

fn event_casket_response_write(
    tk: Token,
    mut tcp_stream: TcpStream,
    mut casket_resp: CasketResponse,
) -> ActionResult {
    use std::io::Write;

    match tcp_stream.write(&casket_resp.response[casket_resp.bytes_sent..]) {
        Ok(sz) => casket_resp.bytes_sent += sz,
        Err(e) => {
            return Err(ActionError {
                token: tk,
                source: ErrorSource::Server,
                error: HttpError::Io(("failed to write casket response to tcp stream", e)),
                tcp_stream,
            })
        }
    }

    if casket_resp.bytes_sent == casket_resp.response.len() {
        Ok(actions::Action::ServerCasketResponseDone((
            tk,
            tcp_stream,
            casket_resp,
        )))
    } else {
        Ok(actions::Action::ServerCasketResponseContinue((
            tk,
            tcp_stream,
            casket_resp,
        )))
    }
}
