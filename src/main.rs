use std::env;
use std::process;
use std::sync::Arc;

use fd_queue::mio::UnixStream;
use fork::fork;
use mio::net::TcpListener;
use ndjsonlogger::{error, info};

#[macro_use]
extern crate lazy_static;

mod config;
mod http;
mod msgs;
mod server;
use server::run_server;
mod worker;
use worker::run_worker;
mod errors;
use errors::{fatal_io_error, RuntimeError, RuntimeResult};
mod pythonexec;

fn main() {
    // argv[1] must be file_name:func_name string for WSGI entry point
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        info!("usage casket filename:function");
        process::exit(0);
    }

    let app_str = &args[1];

    let cfg = match config::Config::from_env() {
        Ok(cfg) => Arc::new(cfg),
        Err(s) => {
            error!("couldn't load config from environment", { error = &s });
            process::exit(1);
        }
    };

    let application = match pythonexec::Application::load(app_str) {
        Ok(app) => app,
        Err(py_err) => {
            error!("couldn't load python application", {
                error = &format!("{}", py_err)
            });
            process::exit(1);
        }
    };

    if let Err(e) = run(cfg, app_str, application) {
        error!("runtime error", { error = &e.reason() });
        process::exit(1);
    }
}

fn run(
    cfg: Arc<config::Config>,
    callable: &str,
    application: pythonexec::Application,
) -> RuntimeResult {
    let listener = TcpListener::bind(cfg.bind_addr)
        .map_err(|err| fatal_io_error("couldn't bind tcp listener on port", err))?;

    let mut parent_socks = vec![];

    for _ in 0..cfg.num_workers {
        let (sock1, sock2) = UnixStream::pair()
            .map_err(|err| fatal_io_error("couldn't create unix socket pair", err))?;

        match fork() {
            Ok(fork::Fork::Parent(pid)) => {
                parent_socks.push((pid, sock1));
            }
            Ok(fork::Fork::Child) => return run_worker(cfg, application, sock2),
            Err(_) => return Err(RuntimeError::ForkFailed),
        }
    }

    drop(application);
    run_server(cfg, callable, listener, parent_socks)
}