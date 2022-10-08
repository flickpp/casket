use std::os::unix::prelude::RawFd;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    CtrlC,
    UnixStreamRead,
    UnixStreamWrite,

    // HTTP Server
    NewStreamFd(RawFd),
    ServerStreamRead,
    QueuedRequests,
    PollPythonResponses,
    ServerStreamWrite,

    RequestReadTimeout,

    CasketResponseWrite,

    PythonCodeTimeout,
}

#[derive(Clone, Copy)]
pub enum Timeout {
    RequestRead,
    PythonCode,
}
