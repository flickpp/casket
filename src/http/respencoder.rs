pub struct ResponseEncoder {
    buffer: Vec<u8>,
}

impl ResponseEncoder {
    pub fn new(code: u16, status: &str) -> Self {
        let mut buffer = Vec::with_capacity(2048);

        buffer.extend(b"HTTP/1.1 ");
        buffer.extend(code.to_string().as_bytes());
        buffer.extend(b" ");
        buffer.extend(status.as_bytes());
        buffer.extend(b" ");
        buffer.extend("\r\n".as_bytes());

        Self { buffer }
    }

    pub fn write_header(&mut self, name: &str, value: &str) {
        self.buffer.extend(name.as_bytes());
        self.buffer.extend(b": ");
        self.buffer.extend(value.as_bytes());
        self.buffer.extend("\r\n".as_bytes());
    }

    pub fn into_buffer(mut self) -> Vec<u8> {
        self.buffer.extend("\r\n".as_bytes());
        self.buffer
    }
}
