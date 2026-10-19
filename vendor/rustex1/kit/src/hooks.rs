//! The JS → Rust bridge.
//!
//! `app.mjs` hands `boot()` a small `hooks` object. It exists so the Rust side
//! can do two things it cannot do on its own:
//!
//! * **capture `this`** — a wasm-exported closure has no notion of a JS
//!   receiver, so `bind(fn)` wraps it as `function (...a) { return fn(this, …a) }`.
//!   Every m-js method defined in Rust takes the scope object as argument 0.
//! * **read the HMR runtime** — `window.__HMR__` is written by the hot client
//!   (which is plain JS); the Rust panels read it back through `stats()`.

use js_sys::Reflect;
use wasm_bindgen::prelude::*;

use crate::js;

thread_local! {
    static HOOKS: std::cell::RefCell<JsValue> = const { std::cell::RefCell::new(JsValue::UNDEFINED) };
}

pub fn install(hooks: JsValue) {
    HOOKS.with(|h| *h.borrow_mut() = hooks);
}

fn hooks() -> JsValue {
    HOOKS.with(|h| h.borrow().clone())
}

pub fn bind(f: &JsValue) -> JsValue {
    let h = hooks();
    if js::is_missing(&h) {
        return f.clone();
    }
    let bound = js::call(&h, "bind", &[f.clone()]);
    if js::is_missing(&bound) {
        f.clone()
    } else {
        bound
    }
}

/// `window.__HMR__` — the hot client's live state.
pub fn stats() -> JsValue {
    js::get(&js::window(), "__HMR__")
}

/// `window.__RUSTEX1__` — the boot token owned by the newest wasm instance.
pub fn current_boot() -> String {
    js::get_str(&js::window(), "__RUSTEX1__")
}

pub fn set_current_boot(token: &str) {
    let w = js::window();
    let _ = Reflect::set(
        &w,
        &JsValue::from_str("__RUSTEX1__"),
        &JsValue::from_str(token),
    );
}

/// `el.dispatchEvent(new CustomEvent(type, { detail, bubbles, composed }))`.
///
/// m-js props flow down and children talk up with custom events; building a
/// `CustomEvent` from wasm means going through the JS bridge rather than
/// reaching for a web-sys feature the kit does not otherwise need.
pub fn dispatch(el: &JsValue, event_type: &str, detail: &JsValue) {
    let h = hooks();
    if js::is_missing(&h) {
        return;
    }
    js::call(
        &h,
        "dispatch",
        &[el.clone(), JsValue::from_str(event_type), detail.clone()],
    );
}

/// Ask the hot client to show its diagnostic panel.
///
/// The template linter is the caller that matters: it has no terminal of its
/// own, so its findings need somewhere on-screen. Rust **compile errors do not
/// come through here** — cargo's stderr is already in the shell running the
/// build, so `hot.mjs` only flashes a line and mirrors the text to the console
/// and `__HMR__.error`. An app that wants the panel for something else may
/// still call it with its own `target`.
pub fn show_overlay(error: &str, target: &str) {
    let h = hooks();
    if !js::is_missing(&h) {
        js::call(
            &h,
            "overlay",
            &[JsValue::from_str(error), JsValue::from_str(target)],
        );
    }
}

pub fn hide_overlay() {
    let h = hooks();
    if !js::is_missing(&h) {
        js::call(&h, "overlay", &[JsValue::NULL, JsValue::NULL]);
    }
}
