use std::fs;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::Builder as ThreadBuilder;

use ndjsonlogger::error;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::config::Config;
use crate::http::{HttpRequest, HttpResponse};
use crate::workq;

mod reqlocal;
mod wsgi;

pub struct Application {
    wsgi_callable: PyObject,
}

type RequestSender = workq::Sender<(usize, HttpRequest)>;
type ResponseReceiver = Receiver<(usize, HttpResponse)>;

pub fn spawn(
    cfg: Arc<Config>,
    application: Application,
    server: (String, u16),
) -> (RequestSender, ResponseReceiver) {
    let (resp_send, resp_recv) = channel();
    let mut req_send = workq::new();

    for n in 0..cfg.num_threads {
        let resp_send = resp_send.clone();
        let server = server.clone();
        let wsgi_callable = Python::with_gil(|py| application.wsgi_callable.clone_ref(py));
        let req_recv = req_send.new_recv();
        let cfg = cfg.clone();

        ThreadBuilder::new()
            .name(format!("python-{}", n))
            .spawn(move || run(cfg, req_recv, resp_send, server, wsgi_callable))
            .expect("couldn't spawn thread");
    }

    (req_send, resp_recv)
}

enum RespBody {
    Memory(Vec<u8>),
    PyIterator(wsgi::BytesIter),
}

fn run(
    cfg: Arc<Config>,
    req_recv: workq::Receiver<(usize, HttpRequest)>,
    resp_send: Sender<(usize, HttpResponse)>,
    server: (String, u16),
    wsgi_callable: PyObject,
) {
    for (key, mut http_req) in req_recv {
        reqlocal::init_req_thread();
        reqlocal::set_context(http_req.context.clone());

        let (body_send, body) = channel();

        let (resp_header, resp_body) = match wsgi::execute(&wsgi_callable, &server, &mut http_req) {
            Ok((resp_header, bytes_iter)) => (resp_header, RespBody::PyIterator(bytes_iter)),
            Err(exec_error) => {
                error!("python application raised exception", {
                    trace_id                   = &http_req.context.trace_id,
                    span_id                    = &http_req.context.span_id,
                    parent_id   : Option<&str> = http_req.context.parent_id_as_ref(),
                    error                      = &exec_error.value,
                    traceback                  = &exec_error.traceback
                });

                let (resp_header, resp_body) = wsgi::handle_wsgi_exec_err(&cfg, exec_error);
                (resp_header, RespBody::Memory(resp_body))
            }
        };

        let http_resp = http_req.into_http_response(resp_header, body);
        if resp_send.send((key, http_resp)).is_err() {
            // Main process has died
            return;
        }

        match resp_body {
            RespBody::Memory(body) => {
                if body_send.send(body).is_err() {
                    // What to do?
                }
            }
            RespBody::PyIterator(mut bytes_iter) => {
                // Stream the response body
                loop {
                    match bytes_iter.next() {
                        Ok(None) => break,
                        Err(_) => {
                            // What to do here? We've already sent the header
                        }
                        Ok(Some(body_chunk)) => {
                            if body_send.send(body_chunk).is_err() {
                                // Again - how to handle!? Main thread has crashed!?
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Application {
    pub fn load(app_str: &str) -> PyResult<Self> {
        let parts = app_str.split(':').collect::<Vec<&str>>();
        if parts.len() != 2 {
            return Err(PyRuntimeError::new_err("invalid python app string"));
        }

        let mod_name = parts[0];
        let func_name = parts[1];

        Python::with_gil(|py| load_application(py, mod_name, func_name))
            .map(|wsgi_callable| Self { wsgi_callable })
    }
}

fn load_application(py: Python, mod_name: &str, func_name: &str) -> PyResult<PyObject> {
    let fname = format!("{}.py", mod_name);
    let code = fs::read_to_string(&fname).map_err(|e| PyRuntimeError::new_err(format!("{}", e)))?;

    let module = PyModule::from_code(py, &code, &fname, mod_name)?;
    module.getattr(func_name).map(|f| f.into())
}
