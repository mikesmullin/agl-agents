//! The rustex1 **demo app** — what a consumer of the kit looks like.
//!
//! Everything reusable lives in `rustex1-kit`; this crate is the application:
//! its stores, its chrome widgets, its pages, its frame loop, and the constants
//! you edit while demoing hot reload.

use rustex1_kit::{self as kit, App, Runtime};
use wasm_bindgen::prelude::*;

mod components;
mod demo;
mod live;
mod pages;
mod store;

struct Rustex1Demo;

impl App for Rustex1Demo {
    fn register_stores(&self, _rt: &mut Runtime) {
        store::register();
    }

    fn register_components(&self, _rt: &mut Runtime) {
        components::register();
    }

    fn register_routes(&self, _rt: &mut Runtime) {
        pages::register();
    }

    fn boot_toast(&self) -> Option<(String, String)> {
        Some((demo::BOOT_TOAST.to_string(), "ok".to_string()))
    }

    fn started(&self, rt: &mut Runtime) {
        live::spawn(rt);
    }
}

/// Called by `kit/web/app.mjs` — on first load and after every wasm rebuild.
#[wasm_bindgen]
pub fn boot(m: JsValue, router: JsValue, hooks: JsValue, opts: JsValue) -> Result<(), JsValue> {
    kit::boot(m, router, hooks, opts, &Rustex1Demo)
}

// ── console surface ────────────────────────────────────────────────────────
// Thin re-exports of the kit's diagnostics so they land on the wasm module the
// browser actually loaded.

/// `__RUSTEX1__MOD.buildInfo()` — rustc version + build stamp.
#[wasm_bindgen(js_name = buildInfo)]
pub fn build_info() -> String {
    kit::build_info()
}

/// `__RUSTEX1__MOD.debugState()` — boot token, frame and tick counters.
#[wasm_bindgen(js_name = debugState)]
pub fn debug_state() -> String {
    kit::debug_state()
}

/// `__RUSTEX1__MOD.dumpTemplate('counterCard')` — the markup m-js was handed.
#[wasm_bindgen(js_name = dumpTemplate)]
pub fn dump_template(name: &str) -> String {
    kit::template_of(name)
}

/// `__RUSTEX1__MOD.lintReport()` — template linter findings.
#[wasm_bindgen(js_name = lintReport)]
pub fn lint_report() -> String {
    kit::lint_report()
}

/// Kick the gauge; the hot client calls it on a finished build.
#[wasm_bindgen]
pub fn pulse() {
    live::pulse();
}
