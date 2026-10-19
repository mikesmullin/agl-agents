//! The rustex1 demo backend.
//!
//! The dev server itself is the shared `rustex1-devserver` crate; an app's
//! `main.rs` is just configuration plus its own JSON routes.

use std::path::Path;

use rustex1_devserver::{args_to_config, Config, Server};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let (config, rebuild) = args_to_config(&args, Config::discover(&start));

    let mut server = Server::new(config)
        // App-specific routes: this is the whole extension point.
        .api("/api/echo", |state, req| {
            let msg = req
                .query
                .split('&')
                .find_map(|kv| kv.strip_prefix("msg="))
                .unwrap_or("hello")
                .to_string();
            json!({
                "echo": msg,
                "from": "rustex1 demo backend",
                "app": state.config.name,
                "requests": state.requests.load(std::sync::atomic::Ordering::Relaxed),
            })
        })
        .api("/api/kit", |_, _| {
            json!({
                "kit": "rustex1-kit",
                "widgets": ["uiBadge", "uiButton", "uiStat", "gaugeBar", "toastStack", "pipelinePanel", "buildFeed"],
                "stores": ["session", "toasts"],
            })
        });

    if rebuild {
        server = server.rebuild_on_start();
    }
    server.run();
}
