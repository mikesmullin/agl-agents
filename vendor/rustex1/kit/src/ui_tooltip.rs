//! `ui-tooltip` — the hover tooltip, ported to Rust from Mike's canonical
//! `tmp/psicom-c-ui/ui-tooltip.mjs` (which is itself the survivor of the
//! mycloud `Tooltip.js` line: "we only need one custom tooltip m-js component").
//!
//! Behaviour is copied deliberately, including the parts that exist because of
//! bugs someone already paid for:
//!
//! * **`position: fixed`, coords measured in JS.** Absolute positioning is
//!   clipped by any `overflow` ancestor, and the bubble must be allowed to
//!   escape its container. Fixed escapes them all.
//! * **hidden via `visibility`/`opacity`, never `display:none`** — layout is
//!   preserved, so `offsetWidth/Height` still read correctly while the bubble is
//!   invisible and `_place()` can measure before it shows.
//! * **`open()`/`close()` place synchronously** on the mouse event. Waiting for a
//!   frame is what makes custom tooltips feel laggy next to native ones.
//! * **a 400 ms tick maintains an open bubble** — re-measures, so it follows
//!   scroll and re-decides the flip as layout moves — and re-asserts hidden when
//!   closed. It writes **exactly one** thing (the label's `textContent`); m-js
//!   owns the bubble, tail and placement classes.
//! * **auto-flip**: prefer the requested side, flip only when that side has no
//!   room and the other does; clamp horizontally to the viewport.
//! * **registered as a factory**, so every instance mints its own `data-ui-tt` id
//!   — a component scope cannot reach its own root element, so the instance tags
//!   its node and looks it up by that id.
//! * **every declared prop has a default**: a host-omitted prop is not a
//!   `with($scope)` binding at all, and the throwing `x-show` would silently drop
//!   the whole bubble.
//!
//! The m-js original's second mode — a live elapsed/expected clock driven by
//! `from`/`expected` — is not ported: nothing in this stack renders one, and the
//! static-label mode is what the segmented controls need.
//!
//! ```ignore
//! // host wraps whatever should be hovered
//! Node::host("ui-tooltip", &[]).bind("label", "o.text").child(the_button)
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;

use crate::h::m;
use crate::js;
use crate::lifecycle;
use crate::mjs::{self, Def};

pub const NAME: &str = "ui-tooltip";

/// Matches the original: re-place an open bubble, re-assert a closed one.
const TICK_MS: f64 = 400.0;
/// Bubble ↔ anchor gap, and the viewport clamp margin (ui-place.mjs).
const GAP: f64 = 9.0;
const MARGIN: f64 = 8.0;

thread_local! {
    /// id → the instance scope, so the shared tick can reach each instance's
    /// props. `destroy` removes the entry.
    static SCOPES: RefCell<HashMap<String, JsValue>> = RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The boot whose shared tick is running. Keyed on the token rather than a bool
    /// so a hot swap starts a fresh chain instead of concluding one is already
    /// going — the old chain stops itself via `am_current_token`, and a bool would
    /// leave the new boot with no timer at all.
    static TICK_FOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn register() {
    // Factory form: called per instance, which is what gives each bubble its own
    // DOM tag id (the original does the same via `++_ttId`).
    mjs::component_factory(NAME, |_props| {
        let id = format!(
            "ui-tt-{}",
            NEXT_ID.with(|n| {
                let v = n.get() + 1;
                n.set(v);
                v
            })
        );
        def(&id)
    });
}

fn def(id: &str) -> JsValue {
    let mut d = Def::new().named(NAME);
    d.prop_str("label", "")
        .prop_str("from", "")
        .prop_str("expected", "")
        .prop_str("placement", "top")
        .prop_bool("multiline", false)
        .prop_bool("hover", false)
        .str("tag", id)
        .tree(
            m("span")
                .class("ui-tt-anchor")
                .on("mouseenter", "open()")
                .on("mouseleave", "close()")
                .kids(vec![
                    m("slot"),
                    m("div")
                        .class("ui-tooltip")
                        .bind("class", "(placement === 'bottom' ? 'ui-tt-bottom' : 'ui-tt-top') + (multiline ? ' ui-tt-multiline' : '')")
                        .child(m("span").attr("class", "ui-tt-label").attr("data-ui-tt", id)),
                ]),
        )
        .method("open", |this, _args| {
            js::set_bool(&this, "hover", true);
            place(&this);
            JsValue::UNDEFINED
        })
        .method("close", |this, _args| {
            js::set_bool(&this, "hover", false);
            place(&this);
            JsValue::UNDEFINED
        })
        .method("init", |this, _args| {
            let tag = js::get_str(&this, "tag");
            SCOPES.with(|s| s.borrow_mut().insert(tag.clone(), this.clone()));
            // one chain for the whole page, not one per bubble — see `ensure_tick`
            ensure_tick();
            tick(&this); // first paint as soon as the span exists
            JsValue::UNDEFINED
        })
        .method("destroy", |this, _args| {
            SCOPES.with(|s| s.borrow_mut().remove(&js::get_str(&this, "tag")));
            JsValue::UNDEFINED
        });
    d.finish()
}

// ---------------------------------------------------------------------------
// measure, place, write
// ---------------------------------------------------------------------------

/// Start the page's single maintenance chain, once per boot.
///
/// Copied from `ui_reltime::ensure_tick`, which proved the design, for the same two
/// reasons:
///
/// **Cost.** Each bubble used to call `lifecycle::every` from its own `init()`, so *N*
/// tooltips meant *N* independent `setTimeout` chains at 400 ms — and a table with a
/// date column carries a tooltip per cell. On the tasks board that was ~35 chains,
/// ~87 callbacks a second, each doing its own `querySelector`. Now: one chain, one
/// walk, one pass.
///
/// **Consistency.** One pass means every bubble on screen is placed against the same
/// layout state. Per-instance chains ran at drifting offsets, so two bubbles sharing
/// an anchor could be measured at different points of the same scroll frame and
/// disagree about where to flip.
///
/// It is a *maintenance* pass, not an animation: an open bubble tracking scroll and a
/// closed one re-asserting `display:none` are the only things it does, which is why
/// 400 ms is enough and why one chain cannot make a bubble lag behind the pointer —
/// `open`/`close` place immediately on hover.
fn ensure_tick() {
    let token = lifecycle::token();
    if TICK_FOR.with(|t| *t.borrow() == Some(token.clone())) {
        return;
    }
    TICK_FOR.with(|t| *t.borrow_mut() = Some(token.clone()));
    let for_cb = token.clone();
    lifecycle::every(TICK_MS, move || {
        // a hot swap supersedes this boot; ticking on would let a dead instance keep
        // writing into the live DOM
        if !lifecycle::am_current_token(&for_cb) {
            return;
        }
        tick_all();
    });
}

/// One tick for every bubble on the page.
///
/// The DOM is the authority on which instances exist: it is walked once and the scope
/// map is only ever probed by tag. That also makes the map self-cleaning — a bubble
/// that x-if removed is never written again, and its scope is dropped here rather
/// than leaking until the next boot. `destroy` still removes eagerly; this is the net
/// for the path where m-js tears a node down without calling it.
fn tick_all() {
    let Some(nodes) = js::query_all("[data-ui-tt]") else {
        return;
    };
    // a NodeList is array-like, not an `Array`, so length and index both come through
    // Reflect
    let count = js_sys::Reflect::get(&nodes, &JsValue::from_str("length"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as usize;
    let mut live: Vec<String> = Vec::with_capacity(count);
    for i in 0..count {
        let node = js_sys::Reflect::get(&nodes, &JsValue::from(i as u32))
            .unwrap_or_else(|_| JsValue::UNDEFINED);
        if node.is_null() || node.is_undefined() {
            continue;
        }
        let tag = js::call_string(&node, "getAttribute", &[JsValue::from_str("data-ui-tt")]);
        if tag.is_empty() {
            continue;
        }
        let scope = SCOPES.with(|s| s.borrow().get(&tag).cloned());
        if let Some(scope) = scope {
            tick(&scope);
            live.push(tag);
        }
    }
    SCOPES.with(|s| {
        let mut m = s.borrow_mut();
        m.retain(|tag, _| live.iter().any(|l| l == tag));
    });
}

/// The original's `_place()`: fixed coords off the anchor, above by default,
/// flip below on short headroom, clamp to the viewport. Every write is
/// changed-only (the sole-writer idiom), and it is safe while hidden because
/// visibility/opacity keep layout.
fn place(scope: &JsValue) {
    let tag = js::get_str(scope, "tag");
    let Some(label) = js::query(&format!("[data-ui-tt=\"{tag}\"]")) else {
        return;
    };
    let bubble = js::get(&label, "parentElement");
    let anchor = js::get(&bubble, "parentElement");
    if js::is_missing(&bubble) || js::is_missing(&anchor) {
        return;
    }

    let hover = js::get_bool(scope, "hover");
    let text = js::get_str(scope, "label");
    let live = !js::get_str(scope, "from").is_empty() && !js::get_str(scope, "expected").is_empty();
    let on = hover && (!text.is_empty() || live);

    let style = js::get(&bubble, "style");
    if !on {
        if js::get_str(&style, "visibility") != "hidden" {
            js::set_str(&style, "visibility", "hidden");
            js::set_str(&style, "opacity", "0");
        }
        return;
    }

    let ar = rect(&anchor);
    let w = js::get_num(&bubble, "offsetWidth");
    let h = js::get_num(&bubble, "offsetHeight");
    let vw = js::get_num(&js::window(), "innerWidth");
    let vh = js::get_num(&js::window(), "innerHeight");

    let room_above = ar.top - GAP - h >= MARGIN;
    let room_below = ar.bottom + GAP + h <= vh - MARGIN;
    let prefer_below = js::get_str(scope, "placement") == "bottom";
    let below = if prefer_below {
        room_below || !room_above
    } else {
        !room_above && room_below
    };
    let top = if below {
        ar.bottom + GAP
    } else {
        ar.top - GAP - h
    };
    let left = (ar.left + ar.width / 2.0 - w / 2.0)
        .max(MARGIN)
        .min(vw - w - MARGIN);

    if js::get_str(&style, "visibility") != "visible" {
        js::set_str(&style, "visibility", "visible");
        js::set_str(&style, "opacity", "1");
    }
    if js::get_str(&style, "position") != "fixed" {
        js::set_str(&style, "position", "fixed");
    }
    let top_px = format!("{top:.0}px");
    let left_px = format!("{left:.0}px");
    if js::get_str(&style, "top") != top_px {
        js::set_str(&style, "top", &top_px);
    }
    if js::get_str(&style, "left") != left_px {
        js::set_str(&style, "left", &left_px);
    }

    // Keep the pip pointing at the anchor even when the bubble was clamped to a
    // viewport edge: the tail's x is the anchor's centre in bubble coordinates.
    let tail_x = format!("{:.0}px", (ar.left + ar.width / 2.0) - left);
    let _ = js::call(
        &style,
        "setProperty",
        &[JsValue::from_str("--tt-tail-x"), JsValue::from_str(&tail_x)],
    );
}

struct Rect {
    top: f64,
    bottom: f64,
    left: f64,
    width: f64,
}

fn rect(el: &JsValue) -> Rect {
    let r = js::call(el, "getBoundingClientRect", &[]);
    Rect {
        top: js::get_num(&r, "top"),
        bottom: js::get_num(&r, "bottom"),
        left: js::get_num(&r, "left"),
        width: js::get_num(&r, "width"),
    }
}

/// The 400 ms maintenance pass: mirror the label (one text write, the tick is
/// that node's sole writer) and re-place, so an open bubble tracks scroll.
fn tick(scope: &JsValue) {
    let tag = js::get_str(scope, "tag");
    if let Some(label) = js::query(&format!("[data-ui-tt=\"{tag}\"]")) {
        let text = js::get_str(scope, "label");
        let from = js::get_str(scope, "from");
        let expected = js::get_str(scope, "expected");
        let want = if !from.is_empty() && !expected.is_empty() {
            // live mode is not ported; fall back to the label
            text.clone()
        } else {
            text.clone()
        };
        if js::get_str(&label, "textContent") != want {
            js::set_str(&label, "textContent", &want);
        }
    }
    place(scope);
}
