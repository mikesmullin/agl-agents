//! # rustex1-devserver
//!
//! The reusable backend half of a rustex1 app: static files over several roots,
//! a small JSON API, the `/__m_hmr` websocket that drives m-js hot reload, the
//! filesystem watcher that rebuilds the wasm frontend, and self-restart when the
//! backend itself changes.
//!
//! An app's `src/main.rs` is meant to be this short:
//!
//! ```ignore
//! use rustex1_devserver::{Config, Server};
//! use serde_json::json;
//!
//! fn main() {
//!     Server::new(Config::discover(&std::env::current_dir().unwrap()))
//!         .api("/api/hello", |state, _req| json!({
//!             "message": "hello from rust",
//!             "requests": state.requests.load(Ordering::Relaxed),
//!         }))
//!         .run();
//! }
//! ```
//!
//! The only built-in routes are `/api/status`, `/api/builds`, `/api/config` and
//! the `/__m_hmr` socket; anything else you add comes from [`Server::api`].

mod build;
mod config;
mod http;
mod hub;
mod state;
mod watcher;

pub use build::{wasm_urls, Outcome};
pub use config::Config;
pub use http::{Request, Response, MAX_BODY};
pub use state::{Activity, BuildRecord, State, Target};

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use state::log;

pub type Json = serde_json::Value;

/// A handler gets the shared state plus the parsed request (method, path, query,
/// and — for write routes — the JSON body), and returns a [`Response`]. A bare
/// `serde_json::Value` is accepted too and becomes a 200.
pub type ApiHandler = Box<dyn Fn(&Arc<State>, &Request) -> Response + Send + Sync + 'static>;

pub struct Server {
    config: Config,
    api: HashMap<String, ApiHandler>,
    force_rebuild: bool,
}

impl Server {
    pub fn new(config: Config) -> Server {
        Server {
            config,
            api: HashMap::new(),
            force_rebuild: false,
        }
    }

    /// Register an app route for any method (`GET`, `POST`, `PUT`, `PATCH`,
    /// `DELETE`). Handlers return a [`Response`], or a `serde_json::Value` for a
    /// plain 200:
    ///
    /// ```ignore
    /// .api("/api/tasks", |state, req| match req.method.as_str() {
    ///     "POST" => match req.json() {
    ///         Some(body) => tasks::create(state, &body),          // 201 / 400
    ///         None => Response::bad_request("expected a JSON body"),
    ///     },
    ///     _ => tasks::list(state, req),                            // 200
    /// })
    /// ```
    pub fn api<F, R>(mut self, path: &str, handler: F) -> Self
    where
        F: Fn(&Arc<State>, &Request) -> R + Send + Sync + 'static,
        R: Into<Response> + Send + 'static,
    {
        self.api
            .insert(path.to_string(), Box::new(move |s, r| handler(s, r).into()));
        self
    }

    /// Rebuild the frontend on start even if nothing looks stale.
    pub fn rebuild_on_start(mut self) -> Self {
        self.force_rebuild = true;
        self
    }

    /// Build, watch, serve. Does not return.
    pub fn run(self) -> ! {
        let has_shell = self
            .config
            .statics
            .iter()
            .any(|root| root.join("index.html").is_file());
        if !has_shell {
            eprintln!(
                "rustex1: no index.html in any static root under {} — is this the app root?",
                self.config.root.display()
            );
            std::process::exit(2);
        }
        for route in self.api.keys() {
            log("api", &format!("GET {route}"));
        }

        let Server {
            config,
            api,
            force_rebuild,
        } = self;
        let state = Arc::new(State::new(config, api));
        log(
            "boot",
            &format!(
                "{} v{} pid={} boot={} root={} wasm={}",
                state.config.name,
                env!("CARGO_PKG_VERSION"),
                std::process::id(),
                state.boot_id,
                state.config.root.display(),
                state.config.app.package,
            ),
        );
        if std::env::var("RUSTEX1_RESTARTED").is_ok() {
            log("boot", "restarted after a backend source change");
        }

        // 1 · the frontend must exist before the browser asks for it.
        if force_rebuild || build::wasm_is_stale(&state) {
            build::rebuild_wasm(&state, &["startup".to_string()]);
        } else {
            log("build", "reusing web/pkg/* (pass --rebuild to force)");
        }

        // 2 · sources.
        watcher::spawn(Arc::clone(&state));

        // 3 · serve.
        let (js, wasm) = state.config.pkg_urls();
        let listener = bind(state.config.port);
        log(
            "http",
            &format!(
                "http://127.0.0.1:{}/  ·  ws /__m_hmr  ·  {js} + {wasm}  ·  {} static root(s)",
                state.config.port,
                state.config.statics.len()
            ),
        );

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let state = Arc::clone(&state);
                    std::thread::spawn(move || {
                        // A panicking connection must not take the server down.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            http::serve(stream, state);
                        }));
                    });
                }
                Err(e) => log("warn", &format!("accept failed: {e}")),
            }
        }

        std::process::exit(0);
    }
}

/// Hand the process over to the freshly built backend binary, keeping the port.
pub fn restart(state: &Arc<State>) {
    build::restart(state)
}

/// After a self-restart the old process may still hold the port for a moment.
fn bind(port: u16) -> TcpListener {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let mut last = None;
    for attempt in 0..50 {
        match TcpListener::bind(addr) {
            Ok(l) => return l,
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(100));
                if attempt == 4 {
                    log(
                        "boot",
                        &format!("port {port} busy, waiting for the old server to exit…"),
                    );
                }
            }
        }
    }
    eprintln!("rustex1: cannot bind {addr}: {}", last.unwrap());
    std::process::exit(1);
}

/// CLI convenience: `--port N`, `--rebuild`, or a bare port number.
pub fn args_to_config(args: &[String], mut config: Config) -> (Config, bool) {
    let mut rebuild = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if let Some(v) = args.get(i + 1) {
                    if let Ok(p) = v.parse() {
                        config.port = p;
                    }
                    i += 1;
                }
            }
            "--rebuild" => rebuild = true,
            "--root" => {
                if let Some(v) = args.get(i + 1) {
                    config.root = PathBuf::from(v);
                    i += 1;
                }
            }
            other => {
                if let Ok(p) = other.parse::<u16>() {
                    config.port = p;
                }
            }
        }
        i += 1;
    }
    (config, rebuild)
}
