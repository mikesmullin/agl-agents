//! The kit's own loops: mirror the hot client's telemetry into `$store.session`.
//!
//! Everything here is **low frequency** and goes through the diff, which is how
//! m-js wants structural change done. High-frequency work (an animation, a live
//! meter) belongs in the app, written as *one* DOM attribute per frame — see
//! `app/src/live.rs` in the rustex1 demo for the pattern.
//!
//! All loops are owned by [`crate::lifecycle`]: a superseded wasm instance stops
//! re-arming itself instead of fighting the new one for the same node.

use crate::hooks;
use crate::js;
use crate::lifecycle;
use crate::mjs;

pub fn spawn() {
    telemetry();
    prune_loop();
}

/// 5 Hz: `window.__HMR__` (written by the JS hot client) → the `session` store,
/// so Rust panels can render HMR state without touching the DOM themselves.
fn telemetry() {
    lifecycle::every(200.0, || {
        let stats = hooks::stats();
        let s = mjs::store_ref("session");
        if js::is_missing(&s) {
            return;
        }

        let connected = js::get_bool(&stats, "connected");
        let building = js::get_bool(&stats, "building");
        let phase = js::get_str(&stats, "phase");
        let file = js::get_str(&stats, "file");
        let swaps = js::get_num(&stats, "swaps");
        let error = js::get_str(&stats, "error");
        let last_build_ms = js::get_num(&stats, "lastBuildMs");

        js::set_bool(&s, "online", connected);
        js::set_bool(&s, "building", building);
        js::set_str(
            &s,
            "state",
            match (connected, building) {
                (false, _) => "offline",
                (true, true) => "rebuilding…",
                (true, false) if phase.is_empty() => "live",
                (true, false) => &phase,
            },
        );
        js::set_str(
            &s,
            "tone",
            match (connected, building, error.is_empty()) {
                (false, _, _) => "err",
                (_, true, _) => "busy",
                (_, _, false) => "warn",
                _ => "ok",
            },
        );
        if !file.is_empty() {
            js::set_str(&s, "lastFile", &file);
        }
        js::set_num(&s, "swaps", swaps);
        if last_build_ms > 0.0 {
            js::set_num(&s, "buildMs", last_build_ms);
        }
        if !error.is_empty() {
            js::set_str(&s, "error", &error);
        }
    });
}

/// 1 Hz housekeeping: expire toasts so a page left open during a long edit
/// session does not accumulate them.
pub fn prune_loop() {
    lifecycle::every(1000.0, || {
        let toasts = mjs::store_ref("toasts");
        js::call(&toasts, "prune", &[]);
    });
}
