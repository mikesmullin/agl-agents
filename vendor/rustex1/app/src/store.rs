//! The demo's own stores — the state a hot swap must not lose.
//!
//! Anything registered here lives on `window` (m-js's store bucket), so editing
//! a `.rs` file merges new methods and **keeps the values**. Anything in a Rust
//! thread-local instead would reset on every rebuild — which is exactly why the
//! counter is a store and the gauge element cache is not.

use wasm_bindgen::prelude::*;

use rustex1_kit::js;
use rustex1_kit::mjs;
use rustex1_kit::mjs::Def;

use crate::demo;

pub fn register() {
    counter();
    meters();
}

fn counter() {
    let mut d = Def::new();
    d.num("count", 0.0)
        .num("clicks", 0.0)
        .str("lastOp", "—")
        .method("inc", |this, args| {
            let step = js::arg_num(&args, 0, demo::COUNTER_STEP);
            js::set_num(&this, "count", js::get_num(&this, "count") + step);
            js::set_num(&this, "clicks", js::get_num(&this, "clicks") + 1.0);
            js::set_str(&this, "lastOp", &format!("inc({step})"));
            JsValue::UNDEFINED
        })
        .method("reset", |this, _args| {
            js::set_num(&this, "count", 0.0);
            js::set_str(&this, "lastOp", "reset");
            JsValue::UNDEFINED
        });
    mjs::store("counter", &d.finish());
}

/// The gauge's numbers. The bar itself is driven by the frame loop in
/// `live.rs` — this store is only the low-frequency label, which the diff owns.
fn meters() {
    let mut d = Def::new();
    d.num("load", 0.0)
        .str("pctText", "0%")
        .num("peak", 0.0)
        .str("phase", "idle");
    mjs::store("meters", &d.finish());
}
