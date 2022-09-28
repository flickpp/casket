use super::Context;

#[derive(Clone)]
pub struct HttpResponse {
    pub code: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub context: Context,
    pub keep_alive: bool,
}
