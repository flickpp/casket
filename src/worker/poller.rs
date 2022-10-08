use std::cmp;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::time;

use mio::{event::Source, Events, Interest, Poll, Token};

use crate::errors::{fatal_io_error, RuntimeResult};

use super::events::Event;

pub struct Poller {
    poll: Poll,
    mio_events: Events,
    events_reg_read: HashMap<Token, Event>,
    events_reg_write: HashMap<Token, Event>,
    timerq: BinaryHeap<cmp::Reverse<TimerEntry>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TimerEntry {
    token: Token,
    timeout: time::SystemTime,
    event: Event,
}

impl cmp::PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        Ord::cmp(&self.timeout, &other.timeout)
    }
}

impl Poller {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            poll: Poll::new()?,
            mio_events: Events::with_capacity(64),
            events_reg_read: HashMap::new(),
            events_reg_write: HashMap::new(),
            timerq: BinaryHeap::new(),
        })
    }

    pub fn tick(
        &mut self,
        events: &mut Vec<(Token, Event)>,
        timeout: Option<time::Duration>,
    ) -> RuntimeResult {
        let now = time::SystemTime::now();

        let timeout = match timeout {
            Some(t) => Some(t),
            None => self
                .timerq
                .peek()
                .map(|e| match e.0.timeout.duration_since(now) {
                    Err(_) => time::Duration::from_millis(20),
                    Ok(d) => d,
                }),
        };

        // Place timed out events in Q
        while let Some(entry) = self.timerq.peek() {
            if entry.0.timeout > now {
                break;
            }

            let entry = self.timerq.pop().unwrap().0;
            events.push((entry.token, entry.event));
        }

        if let Err(e) = self.poll.poll(&mut self.mio_events, timeout) {
            if e.kind() == io::ErrorKind::Interrupted {
                // This works for now see GH-20
                events.push((Token(0), Event::CtrlC));
            } else {
                return Err(fatal_io_error("worker failed to poll", e));
            }
        }

        for mio_ev in &self.mio_events {
            if mio_ev.is_readable() {
                let ev = self
                    .events_reg_read
                    .remove(&mio_ev.token())
                    .expect("couldn't find event in read reg");
                events.push((mio_ev.token(), ev));
            }

            if mio_ev.is_writable() {
                let ev = self
                    .events_reg_write
                    .remove(&mio_ev.token())
                    .expect("couldn't find event in write reg");

                events.push((mio_ev.token(), ev));
            }
        }

        Ok(())
    }

    pub fn register_read<S: Source>(&mut self, s: &mut S, tk: Token, ev: Event) -> io::Result<()> {
        self.events_reg_read.insert(tk, ev);
        self.poll.registry().register(s, tk, Interest::READABLE)
    }

    pub fn deregister<S: Source>(&mut self, s: &mut S) -> io::Result<()> {
        self.poll.registry().deregister(s)
    }

    pub fn register_write<S: Source>(&mut self, s: &mut S, tk: Token, ev: Event) -> io::Result<()> {
        self.events_reg_write.insert(tk, ev);
        self.poll.registry().register(s, tk, Interest::WRITABLE)
    }

    pub fn reregister_write<S: Source>(
        &mut self,
        s: &mut S,
        tk: Token,
        ev: Event,
    ) -> io::Result<()> {
        self.events_reg_write.insert(tk, ev);
        self.poll.registry().reregister(s, tk, Interest::WRITABLE)
    }

    pub fn reregister_read<S: Source>(
        &mut self,
        s: &mut S,
        tk: Token,
        ev: Event,
    ) -> io::Result<()> {
        self.events_reg_read.insert(tk, ev);
        self.poll.registry().reregister(s, tk, Interest::READABLE)
    }

    pub fn reregister_rw<S: Source>(
        &mut self,
        s: &mut S,
        tk: Token,
        read_ev: Event,
        write_ev: Event,
    ) -> io::Result<()> {
        self.events_reg_read.insert(tk, read_ev);
        self.events_reg_write.insert(tk, write_ev);
        self.poll
            .registry()
            .reregister(s, tk, Interest::READABLE | Interest::WRITABLE)
    }

    pub fn timer_event(&mut self, token: Token, timeout: time::SystemTime, event: Event) {
        self.timerq.push(cmp::Reverse(TimerEntry {
            token,
            timeout,
            event,
        }));
    }
}
