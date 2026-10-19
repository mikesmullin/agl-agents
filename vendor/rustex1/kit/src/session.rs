//! The stores the kit owns.
//!
//! `M.store()` keeps its bucket on `window`, so re-registering these definitions
//! after a hot swap **merges the new methods and keeps the data**. That single
//! fact is what makes Rust hot reload feel like real HMR.
//!
//! | store | what it is for |
//! |---|---|
//! | `session` | dev-server + HMR telemetry, mirrored from `window.__HMR__` and `GET /api/*` |
//! | `toasts` | the transient message queue rendered by [`crate::ui`] |
//!
//! Anything an app wants to survive a swap goes in a store of its own — Rust
//! thread-locals are per-wasm-instance and reset on every rebuild.

use js_sys::{Array, Date, Object};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::js;
use crate::mjs;
use crate::mjs::Def;

fn now_ms() -> f64 {
    Date::now()
}

pub fn register() {
    session();
    toasts();
}

// ---------------------------------------------------------------------------

fn session() {
    let mut d = Def::new();
    d.val("api", &Object::new().into())
        .val("builds", &Array::new().into())
        .bool("online", false)
        .bool("building", false)
        .str("state", "connecting")
        .str("tone", "idle")
        .str("lastFile", "—")
        .str("error", "")
        .str("buildLabel", "—")
        .str("buildTone", "idle")
        .num("swaps", 0.0)
        .num("buildMs", 0.0)
        .num("fetches", 0.0)
        .num("bootCount", 0.0)
        .str("bootId", "")
        .num("lintCount", 0.0)
        .str("lintText", "")
        .str("lintRules", "")
        .str("app", "")
        .str("rustTarget", "wasm32-unknown-unknown");
    mjs::store("session", &d.finish());
}

fn toasts() {
    let mut d = Def::new();
    d.val("items", &Array::new().into())
        .num("seq", 0.0)
        .num("ttl", 7000.0)
        .num("max", 5.0)
        .method("push", |this, args| {
            let text = js::arg_str(&args, 0);
            let tone = {
                let t = js::arg_str(&args, 1);
                if t.is_empty() {
                    "info".to_string()
                } else {
                    t
                }
            };

            let seq = js::get_num(&this, "seq") + 1.0;
            js::set_num(&this, "seq", seq);

            let item = Object::new();
            js::set_num(&item, "id", seq);
            js::set_str(&item, "text", &text);
            js::set_str(&item, "tone", &tone);
            js::set_num(&item, "at", now_ms());

            let next = Array::new();
            if let Some(list) = js::get(&this, "items").dyn_ref::<Array>() {
                for i in 0..list.length() {
                    next.push(&list.get(i));
                }
            }
            next.push(&item.into());
            let max = js::get_num(&this, "max").max(1.0) as u32;
            while next.length() > max {
                next.shift();
            }
            // Replace (not mutate) so m-js sees a new value and schedules a draw.
            js::set(&this, "items", &next.into());
            JsValue::UNDEFINED
        })
        .method("dismiss", |this, args| {
            let id = js::arg_num(&args, 0, -1.0);
            let next = Array::new();
            if let Some(list) = js::get(&this, "items").dyn_ref::<Array>() {
                for i in 0..list.length() {
                    let item = list.get(i);
                    if js::get_num(&item, "id") != id {
                        next.push(&item);
                    }
                }
            }
            js::set(&this, "items", &next.into());
            JsValue::UNDEFINED
        })
        .method("prune", |this, _args| {
            let ttl = js::get_num(&this, "ttl");
            let now = now_ms();
            let next = Array::new();
            let mut dropped = 0.0;
            if let Some(list) = js::get(&this, "items").dyn_ref::<Array>() {
                for i in 0..list.length() {
                    let item = list.get(i);
                    if now - js::get_num(&item, "at") <= ttl {
                        next.push(&item);
                    } else {
                        dropped += 1.0;
                    }
                }
            }
            if dropped > 0.0 {
                js::set(&this, "items", &next.into());
            }
            JsValue::UNDEFINED
        })
        .method("clear", |this, _args| {
            js::set(&this, "items", &Array::new().into());
            JsValue::UNDEFINED
        });
    mjs::store("toasts", &d.finish());
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Push a toast from Rust (used by boot, the build pipeline and the API layer).
pub fn toast(text: &str, tone: &str) {
    let s = mjs::store_ref("toasts");
    js::call(
        &s,
        "push",
        &[JsValue::from_str(text), JsValue::from_str(tone)],
    );
}

/// Write one field on a store proxy; m-js coalesces the redraw.
pub fn write(store: &str, key: &str, value: &JsValue) {
    let s = mjs::store_ref(store);
    if !js::is_missing(&s) {
        js::set(&s, key, value);
    }
}

pub fn write_str(store: &str, key: &str, value: &str) {
    write(store, key, &JsValue::from_str(value));
}

pub fn write_num(store: &str, key: &str, value: f64) {
    write(store, key, &JsValue::from(value));
}

pub fn read(store: &str, key: &str) -> JsValue {
    js::get(&mjs::store_ref(store), key)
}

/// Read a numeric field off a store, for app loops that derive from telemetry.
pub fn read_num(store: &str, key: &str) -> f64 {
    let s = mjs::store_ref(store);
    js::get_num(&s, key)
}
