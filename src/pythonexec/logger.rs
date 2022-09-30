use ndjsonloggercore::{Atom, Entry, Level, StdoutOutputter, Value};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyString};

use super::reqlocal;

#[pyfunction]
pub fn info(msg: &str, keys: &PyDict) -> PyResult<()> {
    log(Level::Info, msg, keys)
}

#[pyfunction]
pub fn warn(msg: &str, keys: &PyDict) -> PyResult<()> {
    log(Level::Warn, msg, keys)
}

#[pyfunction]
pub fn error(msg: &str, keys: &PyDict) -> PyResult<()> {
    log(Level::Error, msg, keys)
}

fn log(level: Level, msg: &str, keys: &PyDict) -> PyResult<()> {
    let mut entries: Vec<Entry> = Vec::with_capacity(keys.len());

    for (k, v) in keys.iter() {
        let key: &PyString = k.downcast()?;
        let value = get_value(v)?;

        entries.push(Entry {
            key: key.to_str()?,
            value,
        });
    }

    let ctx = reqlocal::get_context();
    if let Some(ref ctx) = ctx {
        entries.extend([
            Entry {
                key: "trace_id",
                value: Value::Atom(Atom::String(&ctx.trace_id)),
            },
            Entry {
                key: "span_id",
                value: Value::Atom(Atom::String(&ctx.span_id)),
            },
        ]);

        if let Some(parent_id) = ctx.parent_id_as_ref() {
            entries.push(Entry {
                key: "parent_id",
                value: Value::Atom(Atom::String(parent_id)),
            });
        }
    }

    ndjsonloggercore::log(
        None,
        &mut StdoutOutputter::new(),
        msg,
        level,
        entries.into_iter(),
    );

    Ok(())
}

fn get_value(v: &PyAny) -> PyResult<Value<'_, '_>> {
    if let Ok(s) = v.downcast::<PyString>() {
        return Ok(Value::Atom(Atom::String(s.to_str()?)));
    }

    // NOTE: We must try PyBool before i64
    // as True/False extracts to 1/0
    if let Ok(s) = v.downcast::<PyBool>() {
        return Ok(Value::Atom(Atom::Bool(s.is_true())));
    }

    if let Ok(s) = v.extract::<i64>() {
        return Ok(Value::Atom(Atom::Int(s)));
    }

    Err(PyTypeError::new_err("bad type in log tags value"))
}
