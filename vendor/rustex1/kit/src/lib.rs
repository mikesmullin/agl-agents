//! # rustex1-kit
//!
//! The Rust side of a rustex1 app: everything an author writes *in Rust* to
//! drive m-js. An application crate is usually ~60 lines:
//!
//! ```ignore
//! use rustex1_kit::{m, lit, mjs::Def, session, App, Runtime};
//! use wasm_bindgen::prelude::*;
//!
//! struct Tasks1;
//!
//! impl App for Tasks1 {
//!     fn register_routes(&self, rt: &mut Runtime) {
//!         rt.route("/", "tasks1", |_| {
//!             let mut d = Def::new().named("route /");
//!             d.tree(m("h1").child(lit("hello, world")));
//!             d.finish()
//!         });
//!     }
//! }
//!
//! #[wasm_bindgen]
//! pub fn boot(m: JsValue, router: JsValue, hooks: JsValue, opts: JsValue) -> Result<(), JsValue> {
//!     rustex1_kit::boot(m, router, hooks, opts, &Tasks1)
//! }
//! ```
//!
//! `boot()` does, in order: install the panic hook → adopt the `M`/`Router`
//! namespace and the JS bridge → claim the boot token → register the kit stores
//! (`session`, `toasts`) → `App::register_stores` → register the kit widgets →
//! `App::register_components` → `App::register_routes` → **lint every recorded
//! template** → unmount-if-superseded → `M.mount("#app")` → start the dev
//! telemetry polling → `App::started`.
//!
//! Docs: `docs/RUST_M_JS.md` in the rustex1 checkout.

#![allow(dead_code)] // the interop surface is intentionally wider than any one app uses

use wasm_bindgen::prelude::*;

pub mod api;
pub mod h;
pub mod hooks;
pub mod js;
pub mod lifecycle;
pub mod lint;
pub mod live;
pub mod mjs;
pub mod session;
pub mod ui;
pub mod ui_form;
pub mod ui_reltime;
pub mod ui_tooltip;

// Re-export the pieces an app reaches for constantly.
pub use h::{frag, lit, m, Node};
pub use js_sys;
pub use mjs::Def;
pub use session::toast;
pub use ui_form as form;
pub use wasm_bindgen_futures;

/// A rebuild slower than this is reported as "slow" (the dev panel turns amber).
pub const BUILD_BUDGET_MS: f64 = 3000.0;

/// What an app implements to plug into the preset. Everything has a default
/// except the routes — an app with no routes has no pages.
pub trait App {
    /// Stores that components or route templates read while building. Kit
    /// stores (`session`, `toasts`) are registered before this runs.
    fn register_stores(&self, _rt: &mut Runtime) {}

    /// App-specific widgets. The kit's are already registered, so this is for
    /// additions — or to override a kit widget by registering the same name.
    fn register_components(&self, _rt: &mut Runtime) {}

    /// `Router.register` for each page. Called once per boot.
    fn register_routes(&self, _rt: &mut Runtime);

    /// A toast raised on every boot. Return `None` for silence.
    fn boot_toast(&self) -> Option<(String, String)> {
        None
    }

    /// Spawn loops (frame drivers, polling, housekeeping) — always through
    /// [`Runtime::every`] / [`Runtime::on_frame`] so a superseded instance stops.
    fn started(&self, _rt: &mut Runtime) {}
}

/// Handed to every [`App`] hook. A thin, discoverable surface over the kit —
/// an app should not need to touch `js_sys` directly.
pub struct Runtime {
    pub boot_id: String,
    /// `true` when a previous wasm instance already owned the page, i.e. this
    /// boot is a hot swap rather than a first load.
    pub superseded: bool,
}

impl Runtime {
    pub fn component(&self, name: &str, def: &JsValue) {
        mjs::component(name, def);
    }

    pub fn store(&self, name: &str, def: &JsValue) -> JsValue {
        mjs::store(name, def)
    }

    pub fn store_ref(&self, name: &str) -> JsValue {
        mjs::store_ref(name)
    }

    pub fn route<F>(&self, uri: &str, title: &str, make: F)
    where
        F: FnMut(JsValue) -> JsValue + 'static,
    {
        mjs::router_register(uri, title, make);
    }

    pub fn redirect_to(&self, uri: &str) {
        mjs::router_set(uri);
    }

    pub fn href(&self, uri: &str) -> String {
        mjs::router_href(uri)
    }

    pub fn toast(&self, text: &str, tone: &str) {
        session::toast(text, tone);
    }

    /// Self-rescheduling timer owned by this boot.
    pub fn every<F>(&self, ms: f64, f: F)
    where
        F: FnMut() + 'static,
    {
        lifecycle::every(ms, f);
    }

    /// One callback per painted frame, owned by this boot.
    pub fn on_frame<F>(&self, f: F)
    where
        F: FnMut(f64) + 'static,
    {
        lifecycle::on_frame(f);
    }

    /// Am I still the newest instance? Loop bodies should check this.
    pub fn am_current(&self) -> bool {
        lifecycle::am_current()
    }
}

/// Entry point: called by `kit/web/app.mjs` on first load *and* after every wasm
/// rebuild — the cache-busting timestamp doubles as the boot token.
pub fn boot(
    m: JsValue,
    router: JsValue,
    hooks: JsValue,
    opts: JsValue,
    app: &dyn App,
) -> Result<(), JsValue> {
    install_panic_hook();

    let boot_id = js::get_str(&opts, "bootId");
    let superseded = lifecycle::booted_before();

    mjs::install(m, router);
    hooks::install(hooks);
    lifecycle::claim(&boot_id);

    js::log(&format!(
        "[rustex1] wasm boot #{boot_id} · {}",
        option_env!("RUSTC_VERSION").unwrap_or("rustc ?")
    ));

    let mut rt = Runtime {
        boot_id: boot_id.clone(),
        superseded,
    };

    // 1 · state first: components and pages read stores while building.
    session::register();
    app.register_stores(&mut rt);

    // 2 · widgets: kit catalog, then the app's own (which may override).
    ui::register();
    app.register_components(&mut rt);

    // 3 · pages.
    app.register_routes(&mut rt);

    // 4 · check every recorded template before drawing anything.
    lint::run();

    // 5 · mount. A swap tears the rendered tree down first: m-js keeps live
    //     component instances (and therefore their old template strings) unless
    //     they are unmounted. Store data lives on `window`, so it is untouched.
    if superseded {
        mjs::unmount();
    }
    mjs::mount("#app");

    // 6 · side effects: dev-server telemetry, then the app's own loops.
    api::spawn_all();
    live::spawn();
    app.started(&mut rt);

    if let Some((text, tone)) = app.boot_toast() {
        session::toast(&text, &tone);
    }

    Ok(())
}

/// `GET /api/status`, parsed with reflection — no serde on the wasm side.
pub fn fetch_status(callback: impl FnMut(JsValue) + 'static) {
    api::get("/api/status", None, callback);
}

/// The exact template string m-js was handed for a component.
pub fn template_of(name: &str) -> String {
    mjs::template_of(name)
}

/// Linter findings for the recorded templates.
pub fn lint_report() -> String {
    let issues = lint::check_all();
    if issues.is_empty() {
        return "template lint: clean".to_string();
    }
    issues
        .iter()
        .map(|i| i.render())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Live counters, for the status line and for proving the frame loop is owned by
/// exactly one instance.
pub fn debug_state() -> String {
    format!(
        "boot={} frames={} ticks={} lint={}",
        lifecycle::token(),
        lifecycle::frames(),
        lifecycle::ticks(),
        lint::last_count()
    )
}

/// Compiler identity baked in by `build.rs` of the kit.
pub fn build_info() -> String {
    format!(
        "rustex1-kit · {} · {}",
        option_env!("RUSTC_VERSION").unwrap_or("rustc ?"),
        option_env!("BUILD_TIME").unwrap_or("?")
    )
}

fn install_panic_hook() {
    thread_local! {
        static SET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    SET.with(|set| {
        if set.get() {
            return;
        }
        set.set(true);
        std::panic::set_hook(Box::new(|info| {
            js::error(&format!("[rustex1] rust panic: {info}"));
        }));
    });
}
