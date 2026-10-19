//! mail1 — the app: the Gmail/SQLite mirror rendered with rustex1-kit.
//!
//! The page markup lives in [`pages`], the state and the network calls in
//! [`store`], and the document-level hotkeys in [`keys`].

use rustex1_kit::{App, Runtime};
use wasm_bindgen::prelude::*;

mod keys;
mod pages;
mod store;

struct Mail1;

impl App for Mail1 {
    fn register_stores(&self, _rt: &mut Runtime) {
        store::register();
    }

    fn register_components(&self, _rt: &mut Runtime) {
        // No app-specific widgets; the kit's (uiBadge, uiStat, ui-reltime, the
        // form fields, …) are all that the pages compose from.
    }

    fn register_routes(&self, rt: &mut Runtime) {
        _ = rt;
        pages::register();
    }

    fn started(&self, rt: &mut Runtime) {
        store::poll_labels();
        store::poll_rules();
        store::poll();
        rt.every(4000.0, || store::poll());
        keys::install();
    }
}

#[wasm_bindgen]
pub fn boot(m: JsValue, router: JsValue, hooks: JsValue, opts: JsValue) -> Result<(), JsValue> {
    rustex1_kit::boot(m, router, hooks, opts, &Mail1)
}

// ── console surface ────────────────────────────────────────────────────────

/// `__RUSTEX1__MOD.buildInfo()`
#[wasm_bindgen(js_name = buildInfo)]
pub fn build_info() -> String {
    rustex1_kit::build_info()
}

/// `__RUSTEX1__MOD.debugState()`
#[wasm_bindgen(js_name = debugState)]
pub fn debug_state() -> String {
    rustex1_kit::debug_state()
}

/// `__RUSTEX1__MOD.dumpTemplate(name)`
#[wasm_bindgen(js_name = dumpTemplate)]
pub fn dump_template(name: &str) -> String {
    rustex1_kit::template_of(name)
}

/// `__RUSTEX1__MOD.lintReport()`
#[wasm_bindgen(js_name = lintReport)]
pub fn lint_report() -> String {
    rustex1_kit::lint_report()
}
