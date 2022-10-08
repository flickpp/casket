use std::collections::HashMap;
use std::io;
use std::os::unix::io::AsRawFd;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time;

use fd_queue::mio::UnixStream;
use libc::pid_t;
use mio::{net::TcpListener, net::TcpStream, Events, Interest, Poll, Token};
use ndjsonlogger::{debug, info, warn};

use crate::config::Config;
use crate::errors::{fatal_io_error, RuntimeError, RuntimeResult};

mod unixstreams;
use unixstreams::{UnixStream as ServerUnixStream, UnixStreams as ServerUnixStreams};

// Assign a number to each new stream
// Assuming usize is 64 bits, we have a maxmimum of (2^64) / (2^24) = 1_099_511_627_776 streams
const NEW_STREAM_COUNT_INC: usize = 1 << 24;

// Amount to increment counter for a second request on a keep-alive stream
// We have (2^24)/(2^7) = 131_072 possible requests on a single stream.
// Worker may use range [REQUEST_COUNT + 1, REQUEST_COUNT + 127]
// So a single HTTP request may spawn up to 127 additional items (see worker)
const KEEP_ALIVE_COUNT_INC: usize = 1 << 7;

pub fn run_server(
    cfg: Arc<Config>,
    running: Arc<AtomicBool>,
    close_now: Arc<AtomicBool>,
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

    let mut client_stream_count = (NEW_STREAM_COUNT_INC..).step_by(NEW_STREAM_COUNT_INC);

    let mut errors = Vec::with_capacity(32);
    let mut reading_streams = HashMap::new();
    let mut processing_streams = HashMap::<Token, TcpStream>::new();

    let mut run_shutdown = false;
    let mut ctrlc_instant: Option<time::SystemTime> = None;

    // Exit after we've run shutdown and there are no more processing streams
    loop {
        // Close gracefully after a SIGINT
        if run_shutdown && processing_streams.is_empty() {
            break Ok(());
        }

        // SIGINT happened and CTRLC_WAIT_TIME has expired
        if let Some(instant) = ctrlc_instant {
            match instant.elapsed() {
                Err(_) => break Ok(()),
                Ok(elapsed) => {
                    if elapsed >= cfg.ctrlc_wait_time {
                        // Send shutdown to all sockets
                        for (_, tcp_stream) in processing_streams.drain() {
                            tcp_stream.shutdown(std::net::Shutdown::Both).unwrap_or(());
                        }
                        break Ok(());
                    }
                }
            }
        }

        // Second SIGINT - exit now
        if close_now.load(Ordering::SeqCst) {
            break Ok(());
        }

        errors.extend(unix_streams.reregister(poll.registry()));

        for tk in unix_streams.next_stream_tks() {
            let mut tcp_stream = processing_streams
                .remove(&tk)
                .expect("couldn't find processing tream");

            // Increment
            let new_tk = Token(tk.0 + KEEP_ALIVE_COUNT_INC);

            if run_shutdown {
                if let Err(e) = tcp_stream.shutdown(std::net::Shutdown::Both) {
                    errors.push(e);
                }
            } else {
                if let Err(e) =
                    poll.registry()
                        .register(&mut tcp_stream, new_tk, Interest::READABLE)
                {
                    errors.push(e);
                    continue;
                }

                reading_streams.insert(new_tk, tcp_stream);
            }
        }

        for tk in unix_streams.next_stream_close_tks() {
            let tcp_stream = processing_streams
                .remove(&tk)
                .expect("couldn't find processing tream");

            if let Err(e) = tcp_stream.shutdown(std::net::Shutdown::Both) {
                errors.push(e);
            }
        }

        let timeout = if run_shutdown {
            Some(time::Duration::from_millis(100))
        } else {
            None
        };
        let poll_res = poll.poll(&mut events, timeout);

        // Check we're running
        if (poll_res.is_err() || !running.load(Ordering::SeqCst)) && !run_shutdown {
            errors.extend(shutdown(&mut listener, &mut reading_streams, &poll));

            ctrlc_instant = Some(time::SystemTime::now());
            run_shutdown = true;
        }

        for ev in &events {
            if ev.token() == SERVER_TOKEN {
                if let Ok((mut tcp_stream, _)) = listener.accept() {
                    let tk = Token(client_stream_count.next().unwrap());

                    if reading_streams.len() + processing_streams.len() >= cfg.max_conns {
                        // Drop stream now
                        warn!("maximum number of tcp streams exceeded", {
                            "cfg.max_conns": usize = cfg.max_conns
                        });
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

fn shutdown(
    listener: &mut TcpListener,
    reading_streams: &mut HashMap<Token, TcpStream>,
    poll: &Poll,
) -> Vec<io::Error> {
    info!("casket is shutting down");

    let mut io_errs = vec![];

    // Deregister the listener
    if let Err(e) = poll.registry().deregister(listener) {
        io_errs.push(e);
    }

    // shutdown all idle tcp streams
    for (_, mut stream) in reading_streams.drain() {
        let res = poll
            .registry()
            .deregister(&mut stream)
            .and_then(|_| stream.shutdown(std::net::Shutdown::Both));

        if let Err(e) = res {
            io_errs.push(e);
        }
    }

    io_errs
}
