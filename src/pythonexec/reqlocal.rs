use std::cell::RefCell;

use crate::http::Context;

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = RefCell::new(None);
    static RESPONSE_HEADER: RefCell<Option<ResponseHeader>> = RefCell::new(None);
}

pub struct ResponseHeader {
    pub code: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

pub fn set_context(ctx: Context) {
    CONTEXT.with(|c| *(c.borrow_mut()) = Some(ctx));
}

pub fn init_req_thread() {
    CONTEXT.with(|c| *(c.borrow_mut()) = None);
    RESPONSE_HEADER.with(|r| *(r.borrow_mut()) = None);
}

pub fn put_response_header(resp_header: ResponseHeader) {
    RESPONSE_HEADER.with(|r| *(r.borrow_mut()) = Some(resp_header));
}

pub fn take_response_header() -> Option<ResponseHeader> {
    RESPONSE_HEADER.with(|r| r.take().take())
}
