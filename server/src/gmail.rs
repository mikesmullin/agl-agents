//! A from-scratch Gmail REST client: OAuth2 refresh-token auth plus the four
//! `users.messages` verbs this app needs (`list`, `get`, `modify`, `delete`) and
//! the three that make them usable in bulk (`batchModify`, `trash`, `labels`).
//!
//! No googleapis SDK, no generated discovery client — just `ureq`, `serde_json`
//! and the documented REST surface:
//!
//! ```text
//! POST https://oauth2.googleapis.com/token        grant_type=refresh_token
//! GET  /gmail/v1/users/me/messages                list      (?q & maxResults & pageToken)
//! GET  /gmail/v1/users/me/messages/{id}           get       (?format=minimal|metadata|full)
//! POST /gmail/v1/users/me/messages/{id}/modify    modify    {addLabelIds, removeLabelIds}
//! POST /gmail/v1/users/me/messages/batchModify    modify×N  {ids, addLabelIds, removeLabelIds}
//! POST /gmail/v1/users/me/messages/{id}/trash     trash     (recoverable delete)
//! DELETE /gmail/v1/users/me/messages/{id}         delete    (permanent)
//! GET  /gmail/v1/users/me/labels                  labels
//! POST /gmail/v1/users/me/labels                  create a label (Move to a new folder)
//! ```
//!
//! ## Auth
//!
//! The same shape the `google-email` CLI uses — an *installed app* OAuth client
//! whose **refresh token** is already on disk — but done directly: POST the
//! refresh token to the token endpoint, keep the returned access token, and
//! re-mint it when it expires. The refresh token never leaves the machine and
//! never appears in a URL.
//!
//! Credentials are read from `gmail.json` in the project root:
//!
//! ```json
//! { "type": "authorized_user",
//!   "client_id": "…", "client_secret": "…", "refresh_token": "…" }
//! ```
//!
//! …or from the two files the CLI keeps (`credentials.json` + `token.json`),
//! which is the default. Override with `GMAIL_AUTH=/path/to/authorized_user.json`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};
use ureq::Agent;

pub const SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
pub const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Clone, Debug)]
pub struct Creds {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

impl Creds {
    /// Load the OAuth client + refresh token.
    ///
    /// Order matters: an app-local `gmail.json` wins, then the env var, then the
    /// `google-email` CLI's files. Nothing is ever written here.
    pub fn load(root: &std::path::Path) -> Result<Creds, String> {
        if let Some(p) = env_path("GMAIL_AUTH") {
            if p.is_file() {
                return Creds::from_file(&p);
            }
        }
        let local = root.join("gmail.json");
        if local.is_file() {
            return Creds::from_file(&local);
        }
        if let Some(dir) = env_path("GMAIL_DIR") {
            return Creds::from_cli_pair(&dir);
        }
        // The google-email CLI's project, which already holds an authorized_user
        // token for the gmail.modify scope.
        Creds::from_cli_pair(&std::path::PathBuf::from("/workspace/cli/google-email"))
    }

    fn from_file(path: &std::path::Path) -> Result<Creds, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not JSON: {e}", path.display()))?;
        Creds::from_value(&v).ok_or_else(|| {
            format!(
                "{} has no client_id/refresh_token (expected an authorized_user blob)",
                path.display()
            )
        })
    }

    fn from_cli_pair(dir: &std::path::Path) -> Result<Creds, String> {
        let token_path = dir.join("token.json");
        let creds_path = dir.join("credentials.json");
        let token = std::fs::read_to_string(&token_path)
            .map_err(|e| format!("cannot read {}: {e}", token_path.display()))?;
        let tv: Value = serde_json::from_str(&token).map_err(|e| e.to_string())?;
        if let Some(c) = Creds::from_value(&tv) {
            return Ok(c);
        }
        // token.json without a client id/secret: fall back to credentials.json
        let creds: Value =
            serde_json::from_str(&std::fs::read_to_string(&creds_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let installed = creds.get("installed").or_else(|| creds.get("web")).cloned();
        let base = Creds::from_value(&installed.unwrap_or(creds)).ok_or("no client id/secret")?;
        let refresh = tv
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or("token.json has no refresh_token")?
            .to_string();
        Ok(Creds {
            refresh_token: refresh,
            ..base
        })
    }

    fn from_value(v: &Value) -> Option<Creds> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        Some(Creds {
            client_id: s("client_id")?,
            client_secret: s("client_secret")?,
            refresh_token: s("refresh_token")?,
        })
    }
}

fn env_path(key: &str) -> Option<std::path::PathBuf> {
    std::env::var(key).ok().map(std::path::PathBuf::from)
}

/// One access token plus the moment it stops being valid.
struct Cached {
    token: String,
    until: Instant,
}

/// A token bucket shared by every request this process makes to Gmail.
///
/// Gmail's quota for `users.messages` is per **minute** per user, and the way
/// you find out is a 403 mid-sync. Rather than discovering it the hard way, the
/// client spends from a bucket that refills at `per_sec` and refuses to go
/// faster than that — so a pull is bounded by a number you chose, not by
/// however fast 32 threads can push bytes.
struct Bucket {
    tokens: f64,
    last: Instant,
    per_sec: f64,
}

impl Bucket {
    /// Block until a request may be sent.
    fn acquire(&mut self) {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f64() * self.per_sec)
            .min(self.per_sec);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return;
        }
        let wait = (1.0 - self.tokens) / self.per_sec;
        std::thread::sleep(Duration::from_secs_f64(wait));
        self.tokens = 0.0;
        self.last = Instant::now();
    }
}

pub struct Gmail {
    agent: Agent,
    creds: Creds,
    cached: Mutex<Option<Cached>>,
    bucket: Mutex<Bucket>,
}

impl Gmail {
    pub fn new(creds: Creds) -> Gmail {
        Gmail::with_rate(creds, 20.0)
    }

    /// `per_sec` is the sustained request rate: 20/s is 1200/min, comfortably
    /// inside Gmail's per-minute quota, and the knob a pull can raise.
    pub fn with_rate(creds: Creds, per_sec: f64) -> Gmail {
        let per_sec = per_sec.clamp(1.0, 60.0);
        Gmail {
            // One agent, one connection pool: every worker thread reuses the same
            // keep-alive TLS connections to gmail.googleapis.com.
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(20))
                .timeout_read(Duration::from_secs(60))
                .build(),
            creds,
            cached: Mutex::new(None),
            bucket: Mutex::new(Bucket {
                tokens: per_sec,
                last: Instant::now(),
                per_sec,
            }),
        }
    }

    pub fn set_rate(&self, per_sec: f64) {
        let per_sec = per_sec.clamp(1.0, 60.0);
        let mut b = self.bucket.lock().unwrap();
        b.per_sec = per_sec;
        b.tokens = per_sec;
        b.last = Instant::now();
    }

    /// A live access token, minted from the refresh token when the cached one is
    /// gone or within 60s of expiry.
    pub fn token(&self) -> Result<String, String> {
        if let Some(c) = self.cached.lock().unwrap().as_ref() {
            if c.until > Instant::now() {
                return Ok(c.token.clone());
            }
        }
        let body = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            urlenc(&self.creds.client_id),
            urlenc(&self.creds.client_secret),
            urlenc(&self.creds.refresh_token)
        );
        let resp = self
            .agent
            .post(TOKEN_URL)
            .set("content-type", "application/x-www-form-urlencoded")
            .send_string(&body)
            .map_err(|e| format!("token refresh failed: {e}"))?;
        let v: Value = resp.into_json().map_err(|e| format!("token response: {e}"))?;
        let token = v
            .get("access_token")
            .and_then(|t| t.as_str())
            .ok_or("token response had no access_token")?
            .to_string();
        let secs = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
        let until = Instant::now() + Duration::from_secs(secs.saturating_sub(60).max(30));
        *self.cached.lock().unwrap() = Some(Cached {
            token: token.clone(),
            until,
        });
        Ok(token)
    }

    // ---------------------------------------------------------------- verbs

    /// `users.messages.list` — ids only. Returns `(messages, nextPageToken)`.
    pub fn list(
        &self,
        q: &str,
        max_results: u32,
        page_token: Option<&str>,
    ) -> Result<(Vec<(String, String)>, Option<String>), String> {
        let mut qs: Vec<(String, String)> = vec![
            ("maxResults".into(), max_results.to_string()),
            ("q".into(), q.to_string()),
        ];
        if let Some(t) = page_token {
            qs.push(("pageToken".into(), t.to_string()));
        }
        let v = self.call("GET", "/messages", &qs, None)?;
        let out = v
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|m| {
                        (
                            m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            m.get("threadId").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        )
                    })
                    .filter(|(id, _)| !id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let next = v
            .get("nextPageToken")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        Ok((out, next))
    }

    /// `users.messages.get`. `metadata` is enough for the list (subject, from,
    /// snippet, labelIds) and is much smaller than `full`, which is the only
    /// format that carries the body.
    pub fn get(&self, id: &str, format: &str) -> Result<Value, String> {
        self.call(
            "GET",
            &format!("/messages/{id}"),
            &[("format".into(), format.to_string())],
            None,
        )
    }

    /// `users.messages.modify` — note the `/modify` suffix: POSTing to the
    /// message resource itself is a 404, which is easy to guess wrong.
    pub fn modify(
        &self,
        id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<(), String> {
        self.call(
            "POST",
            &format!("/messages/{id}/modify"),
            &[],
            Some(&modify_body(add, remove)),
        )?;
        Ok(())
    }

    /// `users.messages.batchModify` — the same modify for up to `ids.len()`
    /// messages in one request. This is why bulk archive costs one call.
    pub fn batch_modify(
        &self,
        ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> Result<(), String> {
        let mut body = modify_body(add, remove);
        body["ids"] = json!(ids);
        self.call("POST", "/messages/batchModify", &[], Some(&body))?;
        Ok(())
    }

    /// `users.messages.trash` — recoverable, and what the UI's Delete means.
    pub fn trash(&self, id: &str) -> Result<(), String> {
        self.call("POST", &format!("/messages/{id}/trash"), &[], None)?;
        Ok(())
    }

    /// `users.messages.delete` — permanent.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.call("DELETE", &format!("/messages/{id}"), &[], None)?;
        Ok(())
    }

    /// `users.labels.list`.
    pub fn labels(&self) -> Result<Vec<(String, String, String)>, String> {
        let v = self.call("GET", "/labels", &[], None)?;
        Ok(v
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|l| {
                        (
                            l.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            l.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            l.get("labelListVisibility")
                                .and_then(|x| x.as_str())
                                .unwrap_or("labelShow")
                                .to_string(),
                        )
                    })
                    .filter(|(id, _, _)| !id.is_empty())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `users.labels.create` — Move to a folder that does not exist yet.
    pub fn create_label(&self, name: &str) -> Result<(String, String), String> {
        let v = self.call(
            "POST",
            "/labels",
            &[],
            Some(&json!({
                "name": name,
                "labelListVisibility": "labelShow",
                "messageListVisibility": "show",
            })),
        )?;
        Ok((
            v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            v.get("name").and_then(|x| x.as_str()).unwrap_or(name).to_string(),
        ))
    }

    // ---------------------------------------------------------------- transport

    /// One authenticated call, with a bounded retry on the statuses Gmail uses
    /// for "slow down" and "we blinked".
    fn call(&self, method: &str, path: &str, qs: &[(String, String)], body: Option<&Value>) -> Result<Value, String> {
        let url = format!(
            "{API}{path}{}",
            if qs.is_empty() {
                String::new()
            } else {
                format!(
                    "?{}",
                    qs.iter()
                        .map(|(k, v)| format!("{k}={}", urlenc(v)))
                        .collect::<Vec<_>>()
                        .join("&")
                )
            }
        );
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            self.bucket.lock().unwrap().acquire();
            let token = self.token()?;
            let mut req = match method {
                "GET" => self.agent.get(&url),
                "POST" => self.agent.post(&url),
                "DELETE" => self.agent.delete(&url),
                other => return Err(format!("unsupported method {other}")),
            };
            req = req.set("authorization", &format!("Bearer {token}"));
            let sent = match body {
                Some(b) => req.set("content-type", "application/json").send_string(&b.to_string()),
                // Gmail trash/untrash/stop: "The request body must be empty"
                // (reference/rest/v1/users.messages/trash) but the POST still
                // needs Content-Length: 0 or the frontend answers 411.
                // ureq's req.call() sends no length, so send an empty string.
                None if method == "POST" => req.set("Content-Length", "0").send_string(""),
                None => req.call(),
            };
            match sent {
                Ok(r) => {
                    let text = r.into_string().unwrap_or_default();
                    if text.trim().is_empty() {
                        return Ok(json!({}));
                    }
                    return serde_json::from_str(&text).map_err(|e| format!("{method} {path}: bad JSON: {e}"));
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let detail = resp.into_string().unwrap_or_default();
                    let text = error_text(code, &detail);
                    // A quota miss is not a retryable blip: hammering it makes it
                    // worse. Back off long, try once more, and if it still says
                    // no, report it as a quota stop so the caller can leave the
                    // rest of the batch for the next pull.
                    if is_quota(&text) {
                        if attempt <= 2 {
                            std::thread::sleep(Duration::from_secs(12));
                            continue;
                        }
                        return Err(quota_error(&text));
                    }
                    let retryable = code == 429 || code == 500 || code == 502 || code == 503 || code == 504;
                    if retryable && attempt <= 3 {
                        std::thread::sleep(Duration::from_millis(400 * u64::from(attempt)));
                        continue;
                    }
                    return Err(one_line(&text));
                }
                Err(e) => {
                    if attempt <= 2 {
                        std::thread::sleep(Duration::from_millis(300 * u64::from(attempt)));
                        continue;
                    }
                    return Err(format!("{method} {path}: {e}"));
                }
            }
        }
    }
}

fn modify_body(add: &[String], remove: &[String]) -> Value {
    let mut b = json!({});
    if !add.is_empty() {
        b["addLabelIds"] = json!(add);
    }
    if !remove.is_empty() {
        b["removeLabelIds"] = json!(remove);
    }
    b
}

fn error_text(code: u16, raw: &str) -> String {
    // Gmail errors are JSON: {"error":{"code":429,"message":"…","status":"RESOURCE_EXHAUSTED"}}
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            let status = v
                .pointer("/error/status")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            return format!("HTTP {code}{status}: {msg}");
        }
    }
    format!("HTTP {code}: {}", raw.trim())
}

/// The marker a quota stop carries, so a pull loop can halt instead of failing
/// every remaining message.
pub const QUOTA_MARK: &str = "__quota__ ";

pub fn is_quota(text: &str) -> bool {
    text.contains("Quota exceeded") || text.contains("RESOURCE_EXHAUSTED")
}

pub fn quota_error(text: &str) -> String {
    format!("{QUOTA_MARK}{}", one_line(text))
}

pub fn hit_quota(text: &str) -> bool {
    text.starts_with(QUOTA_MARK)
}

/// Collapse a Gmail error to one line so it fits a toast.
fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// Percent-encode for `application/x-www-form-urlencoded`.
fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// payload → body text
// ---------------------------------------------------------------------------

/// Gmail sends message bodies as base64url strings inside a MIME tree.
///
/// `payload.body.data` for a single-part message, `payload.parts[]` (recursively,
/// because `multipart/alternative` nests `text/plain` under `text/html` or the
/// other way round) for everything else. Returns `(text, html)`, either empty.
pub fn extract_body(doc: &Value) -> (String, String) {
    let mut text = String::new();
    let mut html = String::new();
    if let Some(payload) = doc.get("payload") {
        walk(payload, &mut text, &mut html);
    } else if let Some(data) = doc.pointer("/body/data").and_then(|d| d.as_str()) {
        text = decode(data);
    }
    if html.is_empty() && text.is_empty() {
        if let Some(data) = doc.pointer("/body/data").and_then(|d| d.as_str()) {
            text = decode(data);
        }
    }
    (text, html)
}

fn walk(node: &Value, text: &mut String, html: &mut String) {
    let mime = node.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
    if let Some(data) = node.pointer("/body/data").and_then(|d| d.as_str()) {
        let decoded = decode(data);
        match mime {
            "text/html" => {
                if html.is_empty() {
                    *html = decoded;
                }
            }
            _ => {
                if text.is_empty() {
                    *text = decoded;
                }
            }
        }
    }
    if let Some(parts) = node.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            walk(part, text, html);
        }
    }
}

/// base64url, with or without padding, tolerant of the line breaks Gmail
/// sometimes leaves in.
fn decode(data: &str) -> String {
    let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    for engine in [
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::STANDARD,
    ] {
        if let Ok(bytes) = engine.decode(&clean) {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
    String::new()
}
