use std::env;
use std::fs;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::result;

const VERSION: (usize, usize) = (0, 1);

pub struct Config {
    pub num_workers: usize,
    pub num_threads: usize,
    pub bind_addr: SocketAddr,
    pub hostname: String,
    pub max_conns: usize,
    pub max_requests: usize,
    pub body_stacktrace: bool,
    pub version: (usize, usize),
}

impl Default for Config {
    fn default() -> Self {
        let hostname = fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "casket".to_string())
            .trim_end()
            .to_string();

        Self {
            num_workers: 3,
            num_threads: 2,
            bind_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 8080)),
            hostname,
            max_conns: 128,
            max_requests: 12,
            body_stacktrace: true,
            version: VERSION,
        }
    }
}

impl Config {
    pub fn from_env() -> result::Result<Self, String> {
        let mut slf = Self::default();

        for (key, value) in env::vars() {
            match key.as_ref() {
                "CASKET_BIND_ADDR" => {
                    let addr = value
                        .parse::<SocketAddrV4>()
                        .map_err(|e| format!("CASKET_BIND_ADDR invalid - {:?}", e))?;

                    slf.bind_addr = SocketAddr::V4(addr);
                }
                "CASKET_NUM_WORKERS" => {
                    slf.num_workers = value
                        .parse()
                        .map_err(|_| "CASKET_NUM_WORKERS must be positive integer")?;
                }
                "CASKET_NUM_THREADS_PER_WORKER" => {
                    slf.num_threads = value
                        .parse()
                        .map_err(|_| "CASKET_NUM_THREADS_PER-WORKER must be positive integer")?;
                }
                "CASKET_MAX_CONNECTIONS" => {
                    slf.max_conns = value
                        .parse()
                        .map_err(|_| "CASKET_MAX_CONNECTIONS must be positive integer")?;
                }
                "CASKET_RETURN_STACKTRACE_IN_BODY" => {
                    const ERR_STR: &str = "CASKET_RETURN_STACKTRACE_IN_BODY must be 0 or 1";

                    slf.body_stacktrace =
                        value
                            .parse::<usize>()
                            .map_err(|_| ERR_STR)
                            .and_then(|val| {
                                if val == 0 {
                                    Ok(false)
                                } else if val == 1 {
                                    Ok(true)
                                } else {
                                    Err(ERR_STR)
                                }
                            })?;
                }
                _ => {}
            }
        }

        Ok(slf)
    }

    pub fn port(&self) -> u16 {
        self.bind_addr.port()
    }
}
