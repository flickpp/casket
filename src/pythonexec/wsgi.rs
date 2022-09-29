use std::result;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyIterator, PyList, PyString, PyTuple};

use crate::config::Config;
use crate::http::{HttpRequest, HttpResponseHeader as ResponseHeader};
use ndjsonlogger::error;

use super::reqlocal;

pub struct ExecError {
    pub traceback: String,
    pub value: String,
    pub py_err: PyErr,
}

pub type ExecResult<T> = result::Result<T, ExecError>;

pub fn execute(
    wsgi_callable: &PyObject,
    server: &(String, u16),
    http_req: &mut HttpRequest,
) -> ExecResult<(ResponseHeader, BytesIter)> {
    execute_inner(wsgi_callable, server, http_req)
        .map_err(|py_err| Python::with_gil(|py| build_exec_err(py, py_err)))
}

fn execute_inner(
    wsgi_callable: &PyObject,
    server: &(String, u16),
    http_req: &mut HttpRequest,
) -> PyResult<(ResponseHeader, BytesIter)> {
    let start_response = StartResponse {};
    let bytes_iter = Python::with_gil(|py| -> PyResult<Py<PyIterator>> {
        let environ = build_environ(py, server, http_req)?;
        let start_response = Py::new(py, start_response)?;

        wsgi_callable
            .call1(py, (environ, start_response))
            .and_then(|b| PyIterator::from_object(py, b.as_ref(py)))
            .map(|b| b.into())
    })?;

    let response_header = reqlocal::take_response_header()
        .ok_or_else(|| PyRuntimeError::new_err("start_response not called"))?;

    let bytes_iter = BytesIter::new(bytes_iter)?;
    Ok((response_header, bytes_iter))
}

fn build_exec_err(py: Python, py_err: PyErr) -> ExecError {
    let traceback = py_err.traceback(py).map(|t| t.format()).transpose();

    let traceback = match traceback {
        Ok(Some(t)) => t,
        _ => String::from(""),
    };

    let value = py_err.value(py).to_string();

    ExecError {
        traceback,
        value,
        py_err,
    }
}

pub fn handle_wsgi_exec_err(cfg: &Config, err: ExecError) -> (ResponseHeader, Vec<u8>) {
    if cfg.body_stacktrace {
        handle_python_exc_body_stacktrace(err)
    } else {
        handle_python_exc_empty()
    }
}

fn handle_python_exc_body_stacktrace(err: ExecError) -> (ResponseHeader, Vec<u8>) {
    let headers = vec![
        (
            "Content-Length".to_string(),
            err.traceback.as_bytes().len().to_string(),
        ),
        (
            "Content-Type".to_string(),
            "text/plain; charset=UTF-8".to_string(),
        ),
        ("X-Error".to_string(), err.value),
    ];

    let resp_header = ResponseHeader {
        code: 500,
        reason: String::from("Internal Server Error"),
        headers,
    };

    (resp_header, err.traceback.into_bytes())
}

fn handle_python_exc_empty() -> (ResponseHeader, Vec<u8>) {
    let resp_header = ResponseHeader {
        code: 500,
        reason: String::from("Internal Server Error"),
        headers: vec![("Content-Length".to_string(), "0".to_string())],
    };

    (resp_header, vec![])
}

pub struct BytesIter {
    next_val: Option<Vec<u8>>,
    bytes_iter: Py<PyIterator>,
}

impl BytesIter {
    fn new(bytes_iter: Py<PyIterator>) -> PyResult<Self> {
        let next_val = next_body_chunk(&bytes_iter)?;
        Ok(Self {
            next_val,
            bytes_iter,
        })
    }

    pub fn next(&mut self) -> PyResult<Option<Vec<u8>>> {
        let next_val = next_body_chunk(&self.bytes_iter)?;
        let this_val = self.next_val.take();
        self.next_val = next_val;
        Ok(this_val)
    }
}

fn next_body_chunk(iter: &Py<PyIterator>) -> PyResult<Option<Vec<u8>>> {
    Python::with_gil(|py| match iter.as_ref(py).next() {
        None => Ok(None),
        Some(res) => match res {
            Ok(body_chunk) => {
                let bytes: &PyBytes = body_chunk.downcast()?;
                Ok(Some(bytes.as_bytes().to_vec()))
            }
            Err(e) => Err(e),
        },
    })
}

#[pyclass]
struct StartResponse {}

#[pymethods]
impl StartResponse {
    fn __call__(&self, status: &str, headers: &PyList) -> PyResult<()> {
        let mut resp_headers = vec![];

        let (code, reason) = parse_status(status)?;

        for h in headers {
            let h: &PyTuple = h.downcast()?;

            if h.len() != 2 {
                return Err(PyValueError::new_err("headers should be a list of tuples"));
            }

            let key: &PyString = h.get_item(0)?.downcast()?;
            let value: &PyString = h.get_item(1)?.downcast()?;

            resp_headers.push((key.to_string(), value.to_string()));
        }

        reqlocal::put_response_header(ResponseHeader {
            code,
            reason,
            headers: resp_headers,
        });

        Ok(())
    }
}

fn build_environ(
    py: Python,
    server: &(String, u16),
    http_req: &mut HttpRequest,
) -> PyResult<Py<PyDict>> {
    let environ = PyDict::new(py);

    let mut method = http_req.method.to_string();
    method.make_ascii_uppercase();
    environ.set_item("REQUEST_METHOD", method)?;
    environ.set_item("SCRIPT_NAME", http_req.url.path())?;
    environ.set_item("PATH_INFO", http_req.url.path())?;

    if let Some(query_string) = http_req.url.query() {
        environ.set_item("QUERY_STRING", query_string)?;
    }
    if let Some(ref content_type) = http_req.content_type {
        environ.set_item("CONTENT_TYPE", content_type)?;
    }

    if let Some(body) = http_req.body.as_ref() {
        environ.set_item("CONTENT_LENGTH", body.len())?;
    }

    environ.set_item("SERVER_NAME", &server.0)?;
    environ.set_item("SERVER_PORT", server.1)?;

    environ.set_item("SERVER_PROTOCOL", "HTTP/1.1")?;

    // Headers
    for (name, value) in http_req.headers.iter_mut() {
        name.make_ascii_uppercase();
        let name = name.replace('-', "_");
        environ.set_item(format!("HTTP_{}", name), &value[..])?;
    }

    environ.set_item("wsgi.version", (1, 0))?;
    environ.set_item("wsgi.url_scheme", "http")?;

    let body = http_req.body.take().unwrap_or_default();
    environ.set_item("wsgi.input", Py::new(py, WsgiInput::new(body))?)?;

    environ.set_item("wsgi.errors", Py::new(py, WsgiError {})?)?;

    environ.set_item("wsgi.multithread", true)?;
    environ.set_item("wsgi.multiprocess", true)?;
    environ.set_item("wsgi.run_once", false)?;

    // Casket specific
    let trace_ctx = TraceContext {
        trace_id: http_req.context.trace_id.clone(),
        span_id: http_req.context.span_id.clone(),
        parent_id: http_req.context.parent_id.clone(),
    };
    environ.set_item("casket.trace_ctx", Py::new(py, trace_ctx)?)?;

    Ok(environ.into())
}

fn parse_status(status: &str) -> PyResult<(u16, String)> {
    let status_line = format!("HTTP/1.1 {}\r\n", status);
    let mut headers = [httparse::EMPTY_HEADER];
    let mut resp = httparse::Response::new(&mut headers);

    resp.parse(status_line.as_bytes())
        .map_err(|_| PyValueError::new_err("status string given to start_response not valid"))?;

    let code = resp.code.ok_or_else(|| {
        PyValueError::new_err("status string given to start response missing code")
    })?;

    let reason = resp
        .reason
        .ok_or_else(|| PyValueError::new_err("status string given to start response not valid"))?;

    Ok((code, reason.to_string()))
}

#[pyclass]
pub struct WsgiInput {
    body: Vec<u8>,
    pos: usize,
}

impl WsgiInput {
    pub fn new(body: Vec<u8>) -> Self {
        Self { body, pos: 0 }
    }
}

#[pyclass]
struct WsgiInputIter {
    input: Py<WsgiInput>,
}

#[pymethods]
impl WsgiInput {
    #[args(size = "None")]
    fn read(&mut self, py: Python, size: Option<usize>) -> PyResult<Py<PyBytes>> {
        let bytes_remaining = self.body.len() - self.pos;

        let mut size = match size {
            Some(sz) => sz,
            None => bytes_remaining,
        };

        if size > bytes_remaining {
            size = bytes_remaining;
        }

        let data = &self.body[self.pos..(self.pos + size)];
        self.pos += size;

        let bytes = unsafe { PyBytes::from_ptr(py, data.as_ptr(), data.len()) };

        Ok(bytes.into())
    }

    #[args(size = "None")]
    fn readline(&mut self, py: Python, _size: Option<usize>) -> PyResult<Py<PyBytes>> {
        let remaining = &self.body[self.pos..];

        for (count, &byte) in remaining.iter().enumerate() {
            if byte == b'\n' {
                let data = &self.body[self.pos..(self.pos + count)];
                let bytes = unsafe { PyBytes::from_ptr(py, data.as_ptr(), data.len()) };
                self.pos += count;
                return Ok(bytes.into());
            }
        }

        let bytes = unsafe { PyBytes::from_ptr(py, remaining.as_ptr(), remaining.len()) };
        self.pos = self.body.len();
        Ok(bytes.into())
    }

    #[args(size = "None")]
    fn readlines(&mut self, py: Python, size: Option<usize>) -> PyResult<Py<PyList>> {
        let list = PyList::empty(py);

        for n in 1.. {
            let line = self.readline(py, None)?;
            list.append(line)?;

            if let Some(size) = size {
                if size == n {
                    return Ok(list.into());
                }
            }
        }

        Ok(list.into())
    }

    fn __iter__(slf: Py<Self>, py: Python) -> PyResult<Py<WsgiInputIter>> {
        let iter = WsgiInputIter { input: slf };
        Py::new(py, iter)
    }
}

#[pymethods]
impl WsgiInputIter {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__(&mut self, py: Python) -> Option<u8> {
        let mut input = self.input.borrow_mut(py);
        if input.pos == input.body.len() {
            return None;
        }

        let byte = input.body[input.pos];
        input.pos += 1;

        Some(byte)
    }
}

#[pyclass]
pub struct WsgiError {}

#[pymethods]
impl WsgiError {
    fn write(&self, error: &str) -> PyResult<()> {
        use ndjsonlogger::error;

        error!("application error", { error });
        Ok(())
    }

    fn flush(&self) -> PyResult<()> {
        Ok(())
    }

    fn writelines(&self, py: Python, lines: &PyAny) -> PyResult<()> {
        let lines = PyIterator::from_object(py, lines)?;

        for line in lines {
            let line = line?;
            let line: &PyString = line.downcast()?;
            self.write(line.to_str()?)?;
        }

        Ok(())
    }
}

#[pyclass]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
}

#[pymethods]
impl TraceContext {
    #[getter]
    fn trace_id(&self, py: Python) -> Py<PyString> {
        PyString::new(py, &self.trace_id).into()
    }

    #[getter]
    fn span_id(&self, py: Python) -> Py<PyString> {
        PyString::new(py, &self.span_id).into()
    }

    #[getter]
    fn parent_id(&self, py: Python) -> Option<Py<PyString>> {
        self.parent_id.as_ref().map(|s| PyString::new(py, s).into())
    }
}
