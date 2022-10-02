use std::collections::{HashMap, VecDeque};
use std::time;

use mio::Token;

#[derive(Default, Debug)]
pub struct TimersQ {
    // Ordered queue of Token -> Timeout pairs
    queue: VecDeque<(Token, time::SystemTime)>,

    // Most recent value of Timeout for a Token
    map: HashMap<Token, time::SystemTime>,
}

impl TimersQ {
    pub fn peek(&mut self) -> Option<(Token, time::SystemTime)> {
        let (tk, st) = self.queue.front()?;
        if st == self.map.get(tk).expect("couldn't find time in map") {
            Some((*tk, *st))
        } else {
            self.queue.pop_front();
            self.peek()
        }
    }

    pub fn next_timeout(&mut self, now: time::SystemTime) -> Option<(Token, time::SystemTime)> {
        let (tk, st) = self.peek()?;

        if st < now {
            self.map.remove(&tk);
            self.queue.pop_front()
        } else {
            None
        }
    }

    pub fn push_back(&mut self, tk: Token, st: time::SystemTime) {
        self.map.insert(tk, st);
        self.queue.push_back((tk, st));
    }
}
