// A very simple round robin work stealing queue
// We only use the standlard library mpsc primatives

use std::result;
use std::sync::mpsc;

pub fn new<T>() -> Sender<T> {
    Sender {
        pos: 0,
        senders: vec![],
    }
}

pub struct Sender<T> {
    pos: usize,
    senders: Vec<mpsc::Sender<T>>,
}

impl<T> Sender<T> {
    pub fn new_recv(&mut self) -> Receiver<T> {
        let (sx, rx) = mpsc::channel();

        self.senders.push(sx);

        Receiver { inner: rx }
    }

    pub fn send(&mut self, mut t: T) -> result::Result<(), mpsc::SendError<T>> {
        self.pos += 1;

        while !self.senders.is_empty() {
            if self.pos == self.senders.len() {
                self.pos = 0;
            }

            t = match self.senders[self.pos].send(t) {
                Ok(_) => return Ok(()),
                Err(mpsc::SendError(t)) => t,
            };

            self.senders.remove(self.pos);
        }

        Err(mpsc::SendError(t))
    }
}

pub struct Receiver<T> {
    inner: mpsc::Receiver<T>,
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = mpsc::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}
