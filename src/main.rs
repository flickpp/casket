use std::env;
use std::process;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};

use fd_queue::mio::UnixStream;
use fork::fork;
use mio::net::TcpListener;
use ndjsonlogger::{error, info, warn};

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
mod workq;

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

    // Ctrl-C handler in server
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    if let Err(e) = ctrlc::set_handler(move || r.store(false, Ordering::SeqCst)) {
        warn!("failed to register ctrlc handler - no graceful shutdown", {
            error = &format!("{}", e)
        });
    }

    for _ in 0..cfg.num_workers {
        let (sock1, sock2) = UnixStream::pair()
            .map_err(|err| fatal_io_error("couldn't create unix socket pair", err))?;

        match fork() {
            Ok(fork::Fork::Parent(pid)) => {
                parent_socks.push((pid, sock1));
            }
            Ok(fork::Fork::Child) => return run_worker(cfg, running, application, sock2),
            Err(_) => return Err(RuntimeError::ForkFailed),
        }
    }

    info!("casket started", {
        callable,
        ["casket.version"      : usize = [cfg.version.0, cfg.version.1]],
        "cfg.num_workers"      : usize = cfg.num_workers,
        "cfg.num_threads"      : usize = cfg.num_threads,
        "cfg.max_connections"  : usize = cfg.max_conns,
        "cfg.max_requests"     : usize = cfg.max_requests,
        "cfg.return_stacktrace": bool  = cfg.body_stacktrace
    });

    drop(application);

    run_server(cfg, running, listener, parent_socks)?;

    info!("casket closing");
    Ok(())
}
