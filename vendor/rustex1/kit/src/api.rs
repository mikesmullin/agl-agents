//! Talking to the Rust backend.
//!
//! `fetch` is reached through `globalThis` and parsed with `response.json()`, so
//! the frontend needs no serde and no typed request/response graph — the server's
//! JSON is read with the same reflection helpers as everything else.
//!
//! The kit polls the two dev-server endpoints on its own ([`spawn_all`]), which
//! is how any app gets the HMR status panel, the build log and the "is the
//? server up" indicator for free.

use js_sys::{Array, Function, Object, Promise};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use crate::js;
use crate::lifecycle;
use crate::mjs;
use crate::session;
use crate::BUILD_BUDGET_MS;

/// `GET`(or `method`) `path` → parsed JSON into `then`. Failures are logged and
/// mirrored into `$store.session.error`; the callback only sees success.
pub fn get<F>(path: &str, method: Option<&str>, mut then: F)
where
    F: FnMut(JsValue) + 'static,
{
    send(path, method, None, move |v| then(v));
}

/// Any method, optionally with a JSON body (pass a JS object; it is
/// `JSON.stringify`d for you).
pub fn send<F>(path: &str, method: Option<&str>, body: Option<&JsValue>, mut then: F)
where
    F: FnMut(JsValue) + 'static,
{
    let path = path.to_string();
    let method = method.map(str::to_string);
    let body = body.cloned();
    spawn_local(async move {
        match request(&path, method.as_deref(), body.as_ref()).await {
            Ok(v) => then(v),
            Err(e) => fail(&e),
        }
    });
}

/// `POST` a JS object as JSON. `then` sees the parsed response.
pub fn post_json<F>(path: &str, payload: &JsValue, then: F)
where
    F: FnMut(JsValue) + 'static,
{
    send(path, Some("POST"), Some(payload), then);
}

async fn request(
    path: &str,
    method: Option<&str>,
    body: Option<&JsValue>,
) -> Result<JsValue, JsValue> {
    let fetch = js::get(&js::global(), "fetch")
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("no fetch()"))?;

    let resp = match (method, body) {
        (Some(m), Some(payload)) => {
            // Serialize in JS rather than carrying serde across the boundary.
            let text = js_sys::JSON::stringify(payload)?;
            let init = Object::new();
            js::set_str(&init, "method", m);
            // HeadersInit must be an object (or array of pairs) — a bare
            // "name: value" string is rejected by fetch().
            let headers = Object::new();
            js::set_str(&headers, "content-type", "application/json");
            js::set(&init, "headers", &headers.into());
            js::set(&init, "body", &text.into());
            let p = fetch.call2(&JsValue::NULL, &JsValue::from_str(path), &init.into())?;
            JsFuture::from(p.dyn_into::<Promise>()?).await?
        }
        (Some(m), None) => {
            let init = Object::new();
            js::set_str(&init, "method", m);
            let p = fetch.call2(&JsValue::NULL, &JsValue::from_str(path), &init.into())?;
            JsFuture::from(p.dyn_into::<Promise>()?).await?
        }
        _ => {
            let p = fetch.call1(&JsValue::NULL, &JsValue::from_str(path))?;
            JsFuture::from(p.dyn_into::<Promise>()?).await?
        }
    };

    // A 4xx/5xx with a JSON body is still information: handlers conventionally
    // reply `{ok:false, error:"…"}`, and the app needs that text (e.g. "summary
    // is required"). Only network errors or a non-JSON body count as failures.
    let status = js::get_num(&resp, "status");
    let json_p = js::call(&resp, "json", &[]);
    match JsFuture::from(json_p.dyn_into::<Promise>()?).await {
        Ok(v) if status < 400.0 => Ok(v),
        Ok(v) => {
            js::warn(&format!("[rustex1] api: HTTP {status} from {path}"));
            Ok(v)
        }
        Err(_) => Err(JsValue::from_str(&format!("HTTP {status} from {path}"))),
    }
}

fn bump_fetches() {
    let s = mjs::store_ref("session");
    if !js::is_missing(&s) {
        let n = js::get_num(&s, "fetches") + 1.0;
        js::set_num(&s, "fetches", n);
    }
}

fn fail(err: &JsValue) {
    let msg = js::describe(err);
    js::error(&format!("[rustex1] api: {msg}"));
    session::write_str("session", "error", &msg);
    session::write("session", "online", &JsValue::FALSE);
}

// ---------------------------------------------------------------------------
// dev-server endpoints
// ---------------------------------------------------------------------------

pub fn fetch_status() {
    get("/api/status", None, |v| {
        bump_fetches();
        session::write("session", "api", &v);
        session::write("session", "online", &JsValue::TRUE);
        session::write_str("session", "error", "");
        let app = js::get_str(&v, "app");
        if !app.is_empty() {
            session::write_str("session", "app", &app);
        }
    });
}

pub fn fetch_builds() {
    get("/api/builds?limit=8", None, |v| {
        bump_fetches();
        let builds = js::get(&v, "builds");
        session::write("session", "builds", &builds);
        let Some(list) = builds.dyn_ref::<Array>() else {
            return;
        };
        if list.length() == 0 {
            return;
        }
        let last = list.get(0);
        let ms = js::get_num(&last, "ms");
        let ok = js::get_bool(&last, "ok");
        let target = js::get_str(&last, "target");
        session::write_str(
            "session",
            "buildLabel",
            &format!("{} · {}ms", if ok { &target } else { "failed" }, ms as u64),
        );
        let tone = if !ok {
            "err"
        } else if ms > BUILD_BUDGET_MS {
            "warn"
        } else {
            "ok"
        };
        session::write_str("session", "buildTone", tone);
        session::write_num("session", "buildMs", ms);
    });
}

/// `POST /api/rebuild` — the browser asking the backend to run cargo again.
pub fn rebuild_now() {
    session::write("session", "building", &JsValue::TRUE);
    get("/api/rebuild", Some("POST"), move |v| {
        let ms = js::get_num(&v, "ms");
        let ok = js::get_bool(&v, "ok");
        session::write("session", "building", &JsValue::FALSE);
        session::toast(
            &format!(
                "cargo rebuild {} in {}ms",
                if ok { "ok" } else { "failed" },
                ms as u64
            ),
            if ok { "ok" } else { "err" },
        );
        if !ok {
            session::write("session", "error", &js::get(&v, "error"));
        }
    });
}

/// `POST /api/restart` — hand the port to a freshly built server binary.
pub fn restart_server() {
    get("/api/restart", Some("POST"), |_| {
        session::toast("restarting the rust server…", "warn");
    });
}

/// Initial pulls + the low-frequency poll. Structural churn stays cheap; the
/// 60 fps work belongs in the app's own frame loop.
pub fn spawn_all() {
    fetch_status();
    fetch_builds();
    lifecycle::every(2000.0, fetch_status);
    lifecycle::every(3000.0, fetch_builds);
}
