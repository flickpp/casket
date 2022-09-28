use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;

use fd_queue::mio::UnixStream;
use libc::pid_t;
use mio::{net::TcpListener, Events, Interest, Poll, Token};
use ndjsonlogger::debug;

use crate::config::Config;
use crate::errors::{fatal_io_error, RuntimeError, RuntimeResult};

mod unixstreams;
use unixstreams::{UnixStream as ServerUnixStream, UnixStreams as ServerUnixStreams};

pub fn run_server(
    cfg: Arc<Config>,
    _callable: &str,
    mut listener: TcpListener,
    unix_streams: Vec<(pid_t, UnixStream)>,
) -> RuntimeResult {
    const SERVER_TOKEN: Token = Token(0);

    let mut poll =
        Poll::new().map_err(|err| fatal_io_error("server couldn't create poll instance", err))?;
    let mut events = Events::with_capacity(64);

    // Register TcpListener
    poll.registry()
        .register(&mut listener, SERVER_TOKEN, Interest::READABLE)
        .map_err(|err| fatal_io_error("server couldn't register tcp listener", err))?;

    let mut tk_num = 1;
    let mut server_unix_streams = vec![];
    for (_, unix_stream) in unix_streams {
        server_unix_streams.push(ServerUnixStream::new(Token(tk_num), unix_stream));
        tk_num += 1;
    }
    let mut unix_streams = ServerUnixStreams::new(server_unix_streams);

    let mut errors = Vec::with_capacity(32);
    let mut reading_streams = HashMap::new();
    let mut processing_streams = HashMap::new();

    loop {
        errors.extend(unix_streams.reregister(poll.registry()));

        for tk in unix_streams.next_stream_tks() {
            let mut tcp_stream = processing_streams
                .remove(&tk)
                .expect("couldn't find processing tream");
            if let Err(e) = poll
                .registry()
                .register(&mut tcp_stream, tk, Interest::READABLE)
            {
                errors.push(e);
                continue;
            }

            reading_streams.insert(tk, tcp_stream);
        }

        for tk in unix_streams.next_stream_close_tks() {
            processing_streams
                .remove(&tk)
                .expect("couldn't find processing tream");
        }

        if poll.poll(&mut events, None).is_err() {
            // Ctrl-C
            return Ok(());
        }

        for ev in &events {
            if ev.token() == SERVER_TOKEN {
                if let Ok((mut tcp_stream, _)) = listener.accept() {
                    tk_num += 1;
                    let tk = Token(tk_num);

                    if reading_streams.len() + processing_streams.len() >= cfg.max_conns {
                        // Drop stream now
                        continue;
                    }

                    if let Err(err) =
                        poll.registry()
                            .register(&mut tcp_stream, tk, Interest::READABLE)
                    {
                        errors.push(err);
                        continue;
                    }

                    reading_streams.insert(tk, tcp_stream);
                }
                continue;
            }

            if let Some(mut tcp_stream) = reading_streams.remove(&ev.token()) {
                if let Err(e) = poll.registry().deregister(&mut tcp_stream) {
                    errors.push(e);
                    continue;
                }

                unix_streams.msg_send_tcp_stream(ev.token(), tcp_stream.as_raw_fd());
                processing_streams.insert(ev.token(), tcp_stream);
                continue;
            }

            if let Some(unix_stream) = unix_streams.get_mut(ev.token()) {
                if ev.is_readable() {
                    if let Err(e) = unix_stream.read_stream() {
                        errors.push(e);
                    }
                }

                if ev.is_writable() {
                    if let Err(e) = unix_stream.write_stream() {
                        errors.push(e);
                    }
                }

                continue;
            }

            return Err(RuntimeError::UnknownToken);
        }

        for _err in errors.drain(..) {
            debug!("i/o error in loop", { error = &format!("{}", _err) });
        }
    }
}
