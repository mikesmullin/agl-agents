//! A deliberately small HTTP/1.1 server: static files over several roots, a
//! handful of built-in JSON routes, whatever the app registered with
//! [`Server::api`](crate::Server::api), and the `/__m_hmr` websocket upgrade.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::build;
use crate::hub;
use crate::state::{log, State};

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    /// Raw request body, read up to `MAX_BODY` bytes when `Content-Length` is
    /// present. Must be consumed even if unused: closing a socket with unread
    /// input sends a TCP RST and the client loses the response.
    pub body: Vec<u8>,
    pub content_type: String,
    /// `Content-Length` as parsed from the request head.
    pub content_length: usize,
}

/// Largest request body the dev server will read (1 MiB).
pub const MAX_BODY: usize = 1 << 20;

/// An app route's reply: a status plus a JSON body. Plain `serde_json::Value`
/// also works (it converts with status 200), so read-only handlers stay short.
pub struct Response {
    pub status: u16,
    pub body: serde_json::Value,
}

impl Response {
    pub fn new(status: u16, body: serde_json::Value) -> Response {
        Response { status, body }
    }
    pub fn ok(body: serde_json::Value) -> Response {
        Response { status: 200, body }
    }
    pub fn created(body: serde_json::Value) -> Response {
        Response { status: 201, body }
    }
    /// `{"ok": false, "error": msg}` with status 400.
    pub fn bad_request(msg: impl Into<String>) -> Response {
        Response {
            status: 400,
            body: serde_json::json!({ "ok": false, "error": msg.into() }),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Response {
        Response {
            status: 404,
            body: serde_json::json!({ "ok": false, "error": msg.into() }),
        }
    }
    pub fn server_error(msg: impl Into<String>) -> Response {
        Response {
            status: 500,
            body: serde_json::json!({ "ok": false, "error": msg.into() }),
        }
    }
}

impl From<serde_json::Value> for Response {
    fn from(body: serde_json::Value) -> Response {
        Response { status: 200, body }
    }
}

impl Request {
    /// Parse the body as JSON, `None` when absent or malformed.
    pub fn json(&self) -> Option<serde_json::Value> {
        if self.body.is_empty() {
            return None;
        }
        serde_json::from_slice(&self.body).ok()
    }

    /// A query parameter, decoded.
    pub fn param(&self, key: &str) -> Option<String> {
        query_param(&self.query, key)
    }
}

pub fn serve(mut stream: TcpStream, state: Arc<State>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = stream.set_nodelay(true);

    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        match stream.peek(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_head_end(&buf) {
                    break pos;
                }
                if buf.len() > 64 * 1024 {
                    let _ = respond(&mut stream, 431, "text/plain; charset=utf-8", b"", false);
                    return;
                }
            }
            Err(_) => return,
        }
    };

    let text = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let Some(mut req) = parse_request(&text) else {
        let _ = respond(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            b"bad request",
            false,
        );
        return;
    };

    state
        .requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // The HMR socket: hand the connection to the websocket layer. The request
    // bytes stay unread so tungstenite can perform its own handshake parse.
    if req.path == "/__m_hmr" {
        hub::session(stream, state.hub.clone(), state.boot_id.clone());
        return;
    }

    let head_only = req.method == "HEAD";

    // We only *peeked* the request. Consume the header bytes now (see the RST
    // note on Request::body) and then the body, so the socket is always drained
    // before it closes.
    if head_end > 0 {
        let mut drained = vec![0u8; head_end];
        let _ = stream.read_exact(&mut drained);
    }
    if req.content_length > 0 {
        let want = req.content_length.min(MAX_BODY);
        req.body = vec![0u8; want];
        if stream.read_exact(&mut req.body).is_err() {
            return;
        }
    }

    let result = route(&mut stream, &req, &state, head_only);
    if let Err(e) = result {
        log("warn", &format!("{} {} → {}", req.method, req.path, e));
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn parse_request(text: &str) -> Option<Request> {
    let mut lines = text.lines();
    let request_line = lines.next()?.trim();
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_uppercase();
    let target = parts.next()?;

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };

    let mut content_length = 0usize;
    let mut content_type = String::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.parse().unwrap_or(0),
            "content-type" => content_type = value.to_string(),
            _ => {}
        }
    }

    Some(Request {
        method,
        path: percent_decode(&path),
        query,
        body: Vec::new(),
        content_type,
        content_length,
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn route(
    stream: &mut TcpStream,
    req: &Request,
    state: &Arc<State>,
    head_only: bool,
) -> std::io::Result<()> {
    if req.method == "OPTIONS" {
        return respond_extra(
            stream,
            204,
            "text/plain",
            b"",
            head_only,
            &[
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
                ("Access-Control-Allow-Headers", "content-type"),
            ],
        );
    }

    // App-registered routes win over the built-ins of the same name. HEAD is
    // treated as a GET so a header-only probe does not fall through to the SPA
    // shell and come back as text/html.
    if let Some(handler) = state.api.get(&req.path) {
        if matches!(
            req.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
        ) {
            let resp = handler(state, req);
            return json_response(stream, resp.status, &resp.body, head_only);
        }
    }

    match (req.method.as_str(), req.path.as_str()) {
        ("GET" | "HEAD", "/api/status") => {
            let activity = state.activity.lock().unwrap().clone();
            let last_error = state.last_error.lock().unwrap().clone();
            let (js, wasm) = state.config.pkg_urls();
            let json = serde_json::json!({
                "ok": true,
                "app": state.config.name,
                "server": "rustex1-devserver",
                "version": env!("CARGO_PKG_VERSION"),
                "port": state.config.port,
                "bootId": state.boot_id,
                "uptimeS": state.uptime_s(),
                "requests": state.requests.load(std::sync::atomic::Ordering::Relaxed),
                "clients": state.hub.count(),
                "root": state.config.root.display().to_string(),
                "wasmPackage": state.config.app.package,
                "wasmProfile": state.config.app.profile,
                "watching": {
                    "wasm": state.config.app.watch,
                    "server": state.config.server.watch,
                    "static": state.config.static_watch.iter().map(|p| rel(state, p)).collect::<Vec<_>>(),
                },
                "building": activity.map(|a| serde_json::json!({
                    "target": a.target, "trigger": a.trigger,
                    "elapsedMs": State::now_ms() - a.started_ms,
                })),
                "lastError": last_error,
                "artifacts": [js, wasm],
            });
            return json_response(stream, 200, &json, head_only);
        }
        ("GET" | "HEAD", "/api/config") => {
            // What web/app.mjs needs to know before it imports anything.
            let (js, wasm) = state.config.pkg_urls();
            return json_response(
                stream,
                200,
                &serde_json::json!({
                    "app": state.config.name,
                    "pkgJs": js,
                    "pkgWasm": wasm,
                    "hmrPath": "/__m_hmr",
                    "bootId": state.boot_id,
                }),
                head_only,
            );
        }
        ("GET" | "HEAD", "/api/builds") => {
            let limit = query_param(&req.query, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20)
                .min(60);
            let builds = state.builds.lock().unwrap();
            let items: Vec<serde_json::Value> = builds
                .iter()
                .rev()
                .take(limit)
                .map(|b| {
                    serde_json::json!({
                        "seq": b.seq, "target": b.target, "atMs": b.at_ms, "ms": b.duration_ms,
                        "ok": b.ok, "trigger": b.trigger, "error": b.error,
                    })
                })
                .collect();
            return json_response(
                stream,
                200,
                &serde_json::json!({ "builds": items }),
                head_only,
            );
        }
        ("POST", "/api/rebuild") => {
            let out = build::rebuild_wasm(state, &["POST /api/rebuild".to_string()]);
            return json_response(
                stream,
                if out.ok { 200 } else { 500 },
                &serde_json::json!({
                    "ok": out.ok, "ms": out.ms, "changed": out.changed, "error": out.error,
                }),
                head_only,
            );
        }
        ("POST", "/api/restart") => {
            let state = Arc::clone(state);
            let _ = json_response(
                stream,
                200,
                &serde_json::json!({ "ok": true, "restarting": true }),
                head_only,
            );
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                build::restart(&state);
            });
            return Ok(());
        }
        _ => {}
    }

    if req.method != "GET" && req.method != "HEAD" {
        return respond(
            stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
            head_only,
        );
    }

    // Never SPA-fallback an API path: a typo'd route otherwise comes back as
    // 200 + index.html, which surfaces as a JSON parse error instead of a 404.
    if req.path.starts_with("/api/") {
        return json_response(
            stream,
            404,
            &serde_json::json!({
                "ok": false,
                "error": format!("no route for {} {} — register it with Server::api()", req.method, req.path),
            }),
            head_only,
        );
    }

    match static_file(state, &req.path) {
        Some((path, mime)) => {
            let body = std::fs::read(&path)?;
            respond_extra(
                stream,
                200,
                &mime,
                &body,
                head_only,
                &[("Cache-Control", "no-store")],
            )
        }
        None => respond(
            stream,
            404,
            "text/plain; charset=utf-8",
            format!("404 {}\n", req.path).as_bytes(),
            head_only,
        ),
    }
}

fn rel(state: &State, p: &PathBuf) -> String {
    p.strip_prefix(&state.config.root)
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

/// Resolve a URL path against the static roots in priority order — the app's
/// own `web/` first, so it can shadow any kit asset (`app.mjs`, `hot.mjs`,
/// `kit.css`, even `vendor/phosphor/*`) without copying it. Extension-less
/// paths fall back to `index.html` so the m-js Router can run in path mode.
fn static_file(state: &State, url_path: &str) -> Option<(PathBuf, String)> {
    let rel = url_path.trim_start_matches('/');
    if rel.split('/').any(|seg| seg == "..") {
        return None;
    }

    let mut fallback = None;
    for root in &state.config.statics {
        let mut candidate = root.join(rel);
        if candidate.is_dir() {
            candidate = candidate.join("index.html");
        }
        if candidate.is_file() {
            let mime = mime_type(&candidate).to_string();
            return Some((candidate, mime));
        }
        if fallback.is_none() && !PathBuf::new().join(rel).extension().is_some() {
            let index = root.join("index.html");
            if index.is_file() {
                fallback = Some(index);
            }
        }
    }

    fallback.map(|p| {
        let mime = mime_type(&p).to_string();
        (p, mime)
    })
}

fn mime_type(path: &PathBuf) -> &'static str {
    // Getting these right matters: a .mjs or .css served as application/json
    // silently kills module imports and <link rel=stylesheet>.
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn json_response(
    stream: &mut TcpStream,
    status: u16,
    value: &serde_json::Value,
    head: bool,
) -> std::io::Result<()> {
    let body = value.to_string();
    respond_extra(
        stream,
        status,
        "application/json; charset=utf-8",
        body.as_bytes(),
        head,
        &[
            ("Cache-Control", "no-store"),
            ("Access-Control-Allow-Origin", "*"),
        ],
    )
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head: bool,
) -> std::io::Result<()> {
    respond_extra(
        stream,
        status,
        content_type,
        body,
        head,
        &[("Cache-Control", "no-store")],
    )
}

fn respond_extra(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head: bool,
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(body.len() + 256);
    out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", status, reason(status)).as_bytes());
    out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
    out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    for (k, v) in extra {
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    if !head {
        out.extend_from_slice(body);
    }
    stream.write_all(&out)?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}
