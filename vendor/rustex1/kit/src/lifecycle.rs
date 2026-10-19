//! Who owns the browser's animation frame?
//!
//! Every hot swap creates a **brand new wasm instance** — Rust globals do not
//! survive it. So every loop a boot starts must be able to notice that it has
//! been superseded and stop, otherwise the page fills with zombie rAF loops
//! fighting over the same DOM node.
//!
//! The rule is one token on `window`: the newest boot wins, everyone else
//! checks out.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::hooks;
use crate::js;

thread_local! {
    static BOOT: RefCell<String> = const { RefCell::new(String::new()) };
    static FRAMES: Cell<u64> = const { Cell::new(0) };
    static TICKS: Cell<u64> = const { Cell::new(0) };
}

pub fn claim(token: &str) {
    BOOT.with(|b| *b.borrow_mut() = token.to_string());
    hooks::set_current_boot(token);
}

/// Was a wasm instance already driving this page? `true` means this boot is a
/// hot swap, and the mount sequence has to drop the previous instances.
pub fn booted_before() -> bool {
    !hooks::current_boot().is_empty()
}

pub fn token() -> String {
    BOOT.with(|b| b.borrow().clone())
}

/// Am I still the newest instance?
pub fn am_current() -> bool {
    let mine = token();
    !mine.is_empty() && hooks::current_boot() == mine
}

/// Is a token captured earlier still the current one? For a loop that outlives
/// the closure that started it (the tooltip's shared tick, say), the captured
/// token is the thing to compare — `am_current()` would only ever answer for the
/// instance that is running this code.
pub fn am_current_token(wanted: &str) -> bool {
    !wanted.is_empty() && hooks::current_boot() == wanted
}

pub fn frames() -> u64 {
    FRAMES.with(|f| f.get())
}

pub fn ticks() -> u64 {
    TICKS.with(|f| f.get())
}

/// `requestAnimationFrame` loop: `f(now_ms)` once per painted frame, for as
/// long as this boot is the current one.
pub fn on_frame<F>(mut f: F)
where
    F: FnMut(f64) + 'static,
{
    let handle: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));
    let inner = handle.clone();

    let cb = Closure::wrap(Box::new(move |now: JsValue| {
        if !am_current() {
            return; // superseded — simply stop re-arming
        }
        FRAMES.with(|c| c.set(c.get() + 1));
        f(now.as_f64().unwrap_or(0.0));
        if let Some(h) = &*inner.borrow() {
            js::call_global("requestAnimationFrame", &[h.clone()]);
        }
    }) as Box<dyn FnMut(JsValue)>);

    // Leak with the instance: a hot swap drops it along with everything else.
    let js_fn = cb.into_js_value();
    *handle.borrow_mut() = Some(js_fn.clone());
    js::call_global("requestAnimationFrame", &[js_fn]);
}

/// Recurring timer built on self-rescheduling `setTimeout`, so a stale boot
/// quietly stops instead of ticking forever.
pub fn every<F>(ms: f64, mut f: F)
where
    F: FnMut() + 'static,
{
    let handle: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));
    let inner = handle.clone();

    let cb = Closure::wrap(Box::new(move || {
        if !am_current() {
            return;
        }
        TICKS.with(|c| c.set(c.get() + 1));
        f();
        if let Some(h) = &*inner.borrow() {
            js::call_global("setTimeout", &[h.clone(), JsValue::from(ms)]);
        }
    }) as Box<dyn FnMut()>);

    let js_fn = cb.into_js_value();
    *handle.borrow_mut() = Some(js_fn.clone());
    js::call_global("setTimeout", &[js_fn, JsValue::from(ms)]);
}

/// One-shot timer owned by this boot.
pub fn after<F>(ms: f64, mut f: F)
where
    F: FnMut() + 'static,
{
    let fired = Rc::new(Cell::new(false));
    let cb = Closure::wrap(Box::new(move || {
        if fired.get() || !am_current() {
            return;
        }
        fired.set(true);
        f();
    }) as Box<dyn FnMut()>);
    let js_fn = cb.into_js_value();
    js::call_global("setTimeout", &[js_fn, JsValue::from(ms)]);
}
