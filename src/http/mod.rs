use random_fast_rng::{FastRng, Random};

mod respencoder;
pub use respencoder::ResponseEncoder;

mod httpresponse;
pub use httpresponse::HttpResponse;

pub struct HttpRequest {
    pub method: http_types::Method,
    pub headers: Vec<(String, String)>,
    pub context: Context,
    pub body: Vec<u8>,
    pub url: http_types::Url,
    pub content_type: Option<String>,
    pub content_length: Option<usize>,
    pub keep_alive: bool,
}

#[derive(Clone)]
pub struct Context {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
}

impl Context {
    pub fn new() -> Self {
        let mut rng = FastRng::new();
        let trace_id: [u8; 16] = rng.gen();
        let span_id: [u8; 8] = rng.gen();

        Self {
            trace_id: hex::encode(trace_id),
            span_id: hex::encode(span_id),
            parent_id: None,
        }
    }

    pub fn from_vals(trace_id: &str, parent_id: &str) -> Self {
        let mut rng = FastRng::new();
        let span_id: [u8; 8] = rng.gen();

        Self {
            trace_id: trace_id.to_string(),
            parent_id: Some(parent_id.to_string()),
            span_id: hex::encode(span_id),
        }
    }
}
