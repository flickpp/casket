use std::io;
use std::result;

pub enum RuntimeError {
    Io((&'static str, io::Error)),
    PythonThreadsDied,
    ForkFailed,
    UnknownToken,
}

impl RuntimeError {
    pub fn reason(self) -> String {
        use RuntimeError::*;

        match self {
            Io((reason, err)) => format!("{} - {}", reason, err),
            PythonThreadsDied => "python worker threads stopped".to_string(),
            ForkFailed => "fork failed".to_string(),
            UnknownToken => "unknown token".to_string(),
        }
    }
}

pub type RuntimeResult = result::Result<(), RuntimeError>;

pub fn fatal_io_error(reason: &'static str, err: io::Error) -> RuntimeError {
    RuntimeError::Io((reason, err))
}
