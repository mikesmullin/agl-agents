//! Filesystem watcher → build pipeline → websocket notification.
//!
//! Three classes of edit, three different hot-reload behaviours:
//!
//! | you edit                    | what happens                                              |
//! |-----------------------------|-----------------------------------------------------------|
//! | wasm-side sources           | cargo + wasm-bindgen → `{change:/pkg/<lib>.js}` → m-js hot swap, no reload |
//! | backend sources             | cargo → process re-exec → client reconnect + full reload   |
//! | any static asset            | `{change:/…}` → the m-js client's own css/module/document rules |

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::build;
use crate::state::{log, State};

/// Wait this long after the last event before acting, so an editor that writes
/// a file three times produces one build.
const DEBOUNCE: Duration = Duration::from_millis(120);
/// …but never hold back a rebuild for longer than this.
const MAX_WAIT: Duration = Duration::from_millis(800);

pub fn spawn(state: Arc<State>) {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx).expect("could not start fs watcher");

    let mut dirs: Vec<(PathBuf, &str)> = Vec::new();
    if state.config.has_wasm_sources() {
        for d in state.config.app_watch_abs() {
            dirs.push((d, "wasm"));
        }
    }
    for d in state.config.server_watch_abs() {
        dirs.push((d, "server"));
    }
    for d in &state.config.static_watch {
        dirs.push((d.clone(), "static"));
    }

    for (dir, role) in &dirs {
        match watcher.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => log(
                "watch",
                &format!("watching {} ({})", rel(&state, dir), role),
            ),
            Err(e) => log("warn", &format!("cannot watch {}: {e}", dir.display())),
        }
    }

    std::thread::spawn(move || {
        let _watcher = watcher; // must outlive the loop
        loop {
            let mut paths: HashSet<PathBuf> = HashSet::new();
            match rx.recv() {
                Ok(Ok(ev)) => collect(&state, &mut paths, ev),
                Ok(Err(e)) => log("warn", &format!("watcher error: {e}")),
                Err(_) => break, // channel closed
            }

            let started = Instant::now();
            while let Ok(Ok(ev)) = rx.recv_timeout(DEBOUNCE) {
                collect(&state, &mut paths, ev);
                if started.elapsed() > MAX_WAIT {
                    break;
                }
            }

            if !paths.is_empty() {
                act(&state, paths);
            }
        }
        log("watch", "stopped");
    });
}

fn collect(state: &State, out: &mut HashSet<PathBuf>, ev: Event) {
    match ev.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
        _ => return,
    }
    for path in ev.paths {
        if !is_noise(state, &path) {
            out.insert(path);
        }
    }
}

/// Ignore generated output, build dirs and editor scratch files.
fn is_noise(state: &State, path: &Path) -> bool {
    if path.starts_with(&state.config.root.join("target")) {
        return true;
    }
    // wasm-bindgen's own output — reporting it would double every swap.
    if path.components().any(|c| c.as_os_str() == "pkg") {
        return true;
    }
    if path.starts_with(&state.config.root.join("tools")) {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    name.ends_with('~')
        || name.starts_with('.')
        || name.ends_with(".swp")
        || name.ends_with(".rs.bk")
}

fn rel(state: &State, p: &Path) -> String {
    p.strip_prefix(&state.config.root)
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

fn is_source(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    path.extension().and_then(|e| e.to_str()) == Some("rs") || name == "Cargo.toml"
}

#[derive(Default)]
struct Batch {
    app: Vec<String>,
    server: Vec<String>,
    web: Vec<String>,
}

fn act(state: &Arc<State>, paths: HashSet<PathBuf>) {
    let mut batch = Batch::default();

    for path in paths {
        let under = |dirs: &[PathBuf]| dirs.iter().any(|d| path.starts_with(d));

        if under(&state.config.app_watch_abs()) && is_source(&path) {
            batch.app.push(rel(state, &path));
        } else if under(&state.config.server_watch_abs()) && is_source(&path) {
            batch.server.push(rel(state, &path));
        } else {
            // A static asset: the URL is relative to whichever root owns it.
            for root in &state.config.static_watch {
                if let Ok(r) = path.strip_prefix(root) {
                    if r.components().any(|c| c.as_os_str() == "pkg") {
                        break;
                    }
                    batch
                        .web
                        .push(format!("/{}", r.to_string_lossy().replace('\\', "/")));
                    break;
                }
            }
        }
    }

    if !batch.web.is_empty() {
        for url in &batch.web {
            log("hmr", &format!("change → {url}"));
            state.broadcast(serde_json::json!({ "type": "change", "path": url }));
        }
    }

    if !batch.app.is_empty() {
        batch.app.sort();
        build::rebuild_wasm(state, &batch.app);
    }

    if !batch.server.is_empty() {
        batch.server.sort();
        let outcome = build::rebuild_server(state, &batch.server);
        if outcome.ok {
            build::restart(state);
        }
    }
}
