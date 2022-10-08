use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::Arc;
use std::time;

use mio::Token;

use crate::config::Config;
use crate::errors::{RuntimeError, RuntimeResult};
use crate::http::{HttpRequest, HttpResponse};
use crate::pythonexec;
use crate::workq;

use super::actions::{Action, ActionResult};

pub struct PythonThreads {
    queued_reqs: Vec<(Token, HttpRequest)>,
    num_pending_reqs: usize,
    req_send: workq::Sender<(usize, HttpRequest)>,
    resp_recv: mpsc::Receiver<(usize, HttpResponse)>,
    python_code_start_recv: mpsc::Receiver<(usize, time::SystemTime)>,
    timed_out_reqs: HashSet<Token>,
}

impl PythonThreads {
    pub fn new(cfg: Arc<Config>, application: pythonexec::Application) -> Self {
        let server = (cfg.hostname.clone(), cfg.port());
        let (req_send, code_start_recv, resp_recv) = pythonexec::spawn(cfg, application, server);

        Self {
            queued_reqs: vec![],
            num_pending_reqs: 0,
            req_send,
            resp_recv,
            python_code_start_recv: code_start_recv,
            timed_out_reqs: HashSet::new(),
        }
    }

    pub fn queue_http_req(&mut self, tk: Token, http_req: Box<HttpRequest>) {
        self.queued_reqs.push((tk, *http_req));
    }

    pub fn num_pending_reqs(&self) -> usize {
        self.num_pending_reqs
    }

    pub fn has_queued_reqs(&self) -> bool {
        !self.queued_reqs.is_empty()
    }

    pub fn send_queued_requests(&mut self) -> RuntimeResult {
        for (tk, req) in self.queued_reqs.drain(..) {
            self.req_send
                .send((tk.0, req))
                .map_err(|_| RuntimeError::PythonThreadsDied)?;

            self.num_pending_reqs += 1;
        }

        Ok(())
    }

    pub fn timeout_request(&mut self, tk: Token) {
        self.timed_out_reqs.insert(tk);
    }

    pub fn take_responses(&mut self, results: &mut Vec<ActionResult>) -> RuntimeResult {
        // Code Start
        loop {
            match self.python_code_start_recv.try_recv() {
                Ok((tk, st)) => {
                    results.push(Ok(Action::ServerPythonCodeTimeoutNew((Token(tk), st))))
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(RuntimeError::PythonThreadsDied)
                }
            }
        }

        // HTTP Responses
        loop {
            match self.resp_recv.try_recv() {
                Ok((tk, resp)) => {
                    if !self.timed_out_reqs.remove(&Token(tk)) {
                        results.push(Ok(Action::ServerNewResponse((Token(tk), Box::new(resp)))));
                    }

                    self.num_pending_reqs -= 1;
                }
                Err(mpsc::TryRecvError::Empty) => break Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(RuntimeError::PythonThreadsDied)
                }
            }
        }
    }
}
