use std::collections::HashMap;
use std::io;

use mio::{net::TcpListener, net::TcpStream, Poll, Token};
use ndjsonlogger::info;

pub struct ShutdownData<'s> {
    pub listener: &'s mut TcpListener,
    pub poll: &'s Poll,
    pub reading_streams: &'s mut HashMap<Token, TcpStream>,
}

pub fn shutdown_loop(s: ShutdownData<'_>) -> Vec<io::Error> {
    info!("casket is shutting down");

    let (listener, poll, reading_streams) = (s.listener, s.poll, s.reading_streams);

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
