//! Shells out to `cargo` (and the version-pinned `wasm-bindgen` CLI) to rebuild
//! either half of the app, then tells every connected browser what changed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::state::{log, Activity, BuildRecord, State, Target};

/// Serialises builds: the watcher thread and `POST /api/rebuild` must not run
/// two `cargo` invocations over the same target dir at once.
static BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Outcome {
    pub ok: bool,
    pub ms: u128,
    pub error: Option<String>,
    /// Server-relative URLs that changed on disk, e.g. `/pkg/tasks1_app.js`.
    pub changed: Vec<String>,
}

fn cargo_bin() -> String {
    std::env::var("RUSTEX1_CARGO")
        .ok()
        .or_else(|| std::env::var("CARGO").ok())
        .unwrap_or_else(|| "cargo".to_string())
}

fn run(program: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd)
        .args(args)
        // A `cargo run` parent exports these; they mislead the nested build.
        .env_remove("CARGO_MANIFEST_DIR")
        .env_remove("CARGO_PRIMARY_PACKAGE")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTFLAGS")
        .env("CARGO_TARGET_DIR", cwd.join("target"));

    let out = cmd
        .output()
        .map_err(|e| format!("failed to launch `{program} {}`: {e}", args.join(" ")))?;

    let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(stderr)
    } else {
        let mut msg = stderr;
        if msg.contains("can't find crate for `std`") {
            msg.push_str("\n\n  → hint: rustup target add wasm32-unknown-unknown\n");
        }
        Err(tail(msg, 6000))
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len() && !(bytes[j] == b'm' || (0x40..=0x7e).contains(&bytes[j])) {
                j += 1;
            }
            i = j + 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn tail(s: String, max: usize) -> String {
    if s.len() <= max {
        return s.trim().to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…(truncated)\n{}", &s[start..]).trim().to_string()
}

/// Locate the wasm-bindgen CLI: app `tools/`, then the kit's `tools/` (a
/// submodule checkout), then `$RUSTEX1_WASM_BINDGEN`, then `PATH`.
fn wasm_bindgen_bin(state: &State) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let root = &state.config.root;
    for base in [
        root.clone(),
        root.join("vendor/rustex1"),
        root.join("../rustex1"),
        root.join("kit"),
    ] {
        if let Ok(entries) = std::fs::read_dir(base.join("tools")) {
            for entry in entries.flatten() {
                let p = entry.path().join("wasm-bindgen");
                if p.is_file() {
                    candidates.push(p);
                }
            }
        }
    }
    if let Some(p) = candidates.first() {
        return Ok(p.clone());
    }
    if let Ok(env) = std::env::var("RUSTEX1_WASM_BINDGEN") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Ok(p);
        }
    }
    Ok(PathBuf::from("wasm-bindgen")) // last resort: PATH
}

/// The URLs the browser must reload after a wasm rebuild.
pub fn wasm_urls(state: &State) -> Vec<String> {
    let (js, wasm) = state.config.pkg_urls();
    vec![js, wasm]
}

/// Is `web/pkg/*` older than any wasm-side source? Catches edits made while the
/// server was down — most often during its own restart.
pub fn wasm_is_stale(state: &State) -> bool {
    let (js, _) = state.config.pkg_urls();
    let built = std::fs::metadata(state.config.root.join(js.trim_start_matches('/')))
        .ok()
        .and_then(|m| m.modified().ok());
    let newest = state
        .config
        .app_watch_abs()
        .iter()
        .filter_map(|d| newest_source(d, &["rs", "toml", "lock"]))
        .max();
    match (built, newest) {
        (Some(b), Some(n)) => n > b,
        (None, _) => true,
        (_, None) => false,
    }
}

fn newest_source(dir: &Path, exts: &[&str]) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().map(|n| n == "target").unwrap_or(false) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !exts.contains(&ext.as_str()) {
                continue;
            }
            if let Ok(t) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                if newest.map(|n| t > n).unwrap_or(true) {
                    newest = Some(t);
                }
            }
        }
    }
    newest
}

/// Rebuild the Rust → wasm frontend and hot-swap it in every connected browser.
pub fn rebuild_wasm(state: &State, trigger: &[String]) -> Outcome {
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let started = Instant::now();
    let seq = state.next_seq();

    state.set_activity(Some(Activity {
        target: Target::Wasm.as_str().into(),
        trigger: trigger.to_vec(),
        started_ms: State::now_ms(),
    }));
    state.broadcast(serde_json::json!({
        "type": "build", "phase": "start", "seq": seq,
        "target": "wasm", "trigger": trigger,
    }));
    log("build", &format!("wasm #{seq} ← {}", trigger.join(", ")));

    let cfg = &state.config;
    let result = (|| -> Result<Vec<String>, String> {
        run(
            &cargo_bin(),
            &[
                "build",
                "-p",
                &cfg.app.package,
                "--target",
                "wasm32-unknown-unknown",
                "--profile",
                &cfg.app.profile,
            ],
            &cfg.root,
        )?;

        let wasm = cfg.wasm_artifact();
        if !wasm.is_file() {
            return Err(format!("expected artifact missing: {}", wasm.display()));
        }

        let cli = wasm_bindgen_bin(state)?;
        let cli = cli.to_str().ok_or("non-utf8 wasm-bindgen path")?;
        let out_dir = cfg.pkg_dir();
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        run(
            cli,
            &[
                "--target",
                "web",
                "--out-dir",
                out_dir.to_str().ok_or("non-utf8 out dir")?,
                wasm.to_str().ok_or("non-utf8 wasm path")?,
            ],
            &cfg.root,
        )?;
        Ok(wasm_urls(state))
    })();

    finish(state, seq, "wasm", trigger, started, result)
}

/// Rebuild the backend binary. The caller restarts the process on success.
pub fn rebuild_server(state: &State, trigger: &[String]) -> Outcome {
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let started = Instant::now();
    let seq = state.next_seq();

    state.set_activity(Some(Activity {
        target: Target::Server.as_str().into(),
        trigger: trigger.to_vec(),
        started_ms: State::now_ms(),
    }));
    state.broadcast(serde_json::json!({
        "type": "build", "phase": "start", "seq": seq,
        "target": "server", "trigger": trigger,
    }));
    log("build", &format!("server #{seq} ← {}", trigger.join(", ")));

    let result = (|| -> Result<Vec<String>, String> {
        run(
            &cargo_bin(),
            &["build", "-p", &state.config.server.package],
            &state.config.root,
        )?;
        Ok(vec![])
    })();

    finish(state, seq, "server", trigger, started, result)
}

fn finish(
    state: &State,
    seq: u64,
    target: &str,
    trigger: &[String],
    started: Instant,
    result: Result<Vec<String>, String>,
) -> Outcome {
    let ms = started.elapsed().as_millis();
    match result {
        Ok(changed) => {
            state.set_error(None);
            state.set_activity(None);
            state.record_build(BuildRecord {
                seq,
                target: target.into(),
                at_ms: State::now_ms(),
                duration_ms: ms,
                ok: true,
                trigger: trigger.to_vec(),
                error: None,
            });
            state.broadcast(serde_json::json!({
                "type": "build", "phase": "ok", "seq": seq,
                "target": target, "ms": ms, "changed": changed,
            }));
            for path in &changed {
                // The message m-js's hot client actually acts on.
                state.broadcast(serde_json::json!({ "type": "change", "path": path }));
            }
            log("build", &format!("{target} #{seq} ok in {ms} ms"));
            Outcome {
                ok: true,
                ms,
                error: None,
                changed,
            }
        }
        Err(error) => {
            state.set_error(Some(error.clone()));
            state.set_activity(None);
            state.record_build(BuildRecord {
                seq,
                target: target.into(),
                at_ms: State::now_ms(),
                duration_ms: ms,
                ok: false,
                trigger: trigger.to_vec(),
                error: Some(error.clone()),
            });
            state.broadcast(serde_json::json!({
                "type": "build", "phase": "error", "seq": seq,
                "target": target, "ms": ms, "error": error,
            }));
            log("build", &format!("{target} #{seq} FAILED in {ms} ms"));
            Outcome {
                ok: false,
                ms,
                error: Some(error),
                changed: vec![],
            }
        }
    }
}

/// Hand the process over to the freshly built binary, keeping the port.
pub fn restart(state: &State) {
    // `current_exe()` reports `…/mail1-server (deleted)` as soon as cargo has
    // replaced the binary under a running server — which is precisely the moment
    // a rebuild-then-restart happens, so the hand-over failed every time and the
    // old code kept serving. Fall back to where cargo just wrote the binary.
    let exe = match std::env::current_exe() {
        Ok(e) if e.exists() => e,
        _ => state
            .config
            .root
            .join("target")
            .join("debug")
            .join(&state.config.server.package),
    };
    if !exe.exists() {
        log(
            "fatal",
            &format!("cannot find the server binary at {} — staying up", exe.display()),
        );
        return;
    }
    state.broadcast(serde_json::json!({ "type": "restart", "target": "server" }));
    log("server", &format!("restarting → {}", exe.display()));
    std::thread::sleep(std::time::Duration::from_millis(150));

    let args: Vec<String> = std::env::args().skip(1).collect();
    match Command::new(&exe)
        .args(&args)
        .env("RUSTEX1_RESTARTED", "1")
        .spawn()
    {
        Ok(_child) => std::process::exit(0),
        Err(e) => log("fatal", &format!("restart failed: {e} — staying up")),
    }
}
