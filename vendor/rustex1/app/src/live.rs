//! The fast half of the demo UI, in the shape m-js wants fast UIs shaped.
//!
//! Per frame this module writes **exactly one DOM attribute** — the `width` of
//! the gauge fill rect — through a cached element handle. Everything structural
//! (the `%` label, toasts, panel content) stays in the virtual DOM at a human
//! frequency, via store writes.
//!
//! Both loops are owned by `rustex1_kit::lifecycle`: each tick compares the boot
//! token on `window` before re-arming, so a superseded wasm instance stops
//! animating instead of fighting the new one for the same node.

use std::cell::{Cell, RefCell};

use wasm_bindgen::prelude::*;

use rustex1_kit::{hooks, js, lifecycle, session, Runtime};

const GAUGE_W: f64 = 240.0;
const GAUGE_SELECTOR: &str = "[data-gauge=\"pipeline\"]";

thread_local! {
    static GAUGE: RefCell<Option<JsValue>> = const { RefCell::new(None) };
    static GAUGE_TOKEN: RefCell<String> = const { RefCell::new(String::new()) };
    /// 0..1 injected by a rebuild, decays every frame.
    static BURST: Cell<f64> = const { Cell::new(0.0) };
    static LAST_PULSE: Cell<f64> = const { Cell::new(-1.0) };
}

pub fn spawn(_rt: &mut Runtime) {
    frame_loop();
    meters_label();
}

/// `0..1` — a smooth two-tone signal plus a burst whenever the server rebuilt.
fn signal(t: f64) -> f64 {
    let base =
        0.46 + 0.22 * (t * 1.1).sin() + 0.11 * (t * 2.7 + 1.3).sin() + 0.05 * (t * 6.1 + 0.4).sin();
    let burst = BURST.with(|b| b.get());
    (base + burst * 0.45).clamp(0.0, 1.0)
}

fn gauge_element() -> Option<JsValue> {
    let token = lifecycle::token();
    let cached_ok =
        GAUGE_TOKEN.with(|t| *t.borrow() == token) && GAUGE.with(|g| g.borrow().is_some());
    if cached_ok {
        return GAUGE.with(|g| g.borrow().clone());
    }
    // New boot → the tree was re-rendered; look the node up once and cache it.
    let found = js::query(GAUGE_SELECTOR);
    GAUGE.with(|g| *g.borrow_mut() = found.clone());
    GAUGE_TOKEN.with(|t| *t.borrow_mut() = token);
    found
}

fn frame_loop() {
    lifecycle::on_frame(move |now_ms| {
        let t = now_ms / 1000.0;
        let load = signal(t);
        BURST.with(|b| b.set((b.get() - 0.012).max(0.0)));

        if let Some(el) = gauge_element() {
            // one attribute, one write — no vdom involvement
            js::set_attr(&el, "width", &format!("{:.1}", load * GAUGE_W));
        }
    });
}

/// 5 Hz: the label the diff renders, and the pulse counter that ties the
/// animation to real rebuilds (`hot.mjs` bumps `__HMR__.pulse` on each one).
fn meters_label() {
    lifecycle::every(200.0, || {
        let stats = hooks::stats();
        let pulse = js::get_num(&stats, "pulse");
        if pulse != LAST_PULSE.with(|p| p.get()) {
            LAST_PULSE.with(|p| p.set(pulse));
            BURST.with(|b| b.set(1.0));
        }

        let load = signal(js_sys::Date::now() / 1000.0);
        session::write_num("meters", "load", load);
        session::write_str(
            "meters",
            "pctText",
            &format!("{:.0}% · {}", load * 100.0, crate::demo::GAUGE_LABEL),
        );
        if load > session::read_num("meters", "peak") {
            session::write_num("meters", "peak", load);
        }
    });
}

/// A rebuild happened: kick the gauge so the page visibly reacts.
pub fn pulse() {
    BURST.with(|b| b.set(1.0));
}
