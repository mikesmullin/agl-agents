//! The demo's own widgets — the ones that are about *this* app rather than
//! reusable UI. Everything generic lives in `rustex1_kit::ui`.
//!
//! | name | what it is |
//! |---|---|
//! | `navBar` | brand + router nav + the live HMR badge |
//! | `counterCard` | the store-survives-a-swap demo |
//! | `hintStrip` | the "what just happened" list, fed by `demo::HINTS` |

use js_sys::{Array, Object};
use wasm_bindgen::prelude::*;

use rustex1_kit::h::{lit, m, Node};
use rustex1_kit::js;
use rustex1_kit::mjs::{self, Def};

use crate::demo;

/// Kit widgets + app widgets.
pub const COUNT: usize = rustex1_kit::ui::COUNT + 3;

pub fn register() {
    nav_bar();
    counter_card();
    hint_strip();
    js::log(&format!("[rustex1] registered {} components", COUNT));
}

// ---------------------------------------------------------------------------

/// Router-driven nav. Href strings are baked from `Router.href()` at boot, so
/// path mode and a hard refresh agree; the active state is a method so it stays
/// correct as you navigate.
fn nav_bar() {
    let mut d = Def::new().named("navBar");
    let links = [
        ("/", "Home", "house"),
        ("/storybook", "Storybook", "square-split-horizontal"),
        ("/protocol", "Protocol", "brackets-curly"),
        ("/counter/7", "Params", "hash"),
    ];
    let routes = Array::new();
    for (uri, label, icon) in links {
        let r = Object::new();
        js::set_str(&r, "href", &mjs::router_href(uri));
        js::set_str(&r, "uri", uri);
        js::set_str(&r, "label", label);
        js::set_str(&r, "icon", icon);
        routes.push(&r.into());
    }
    d.val("routes", &routes.into())
        .str("title", "rustex1")
        .tree(m("header").class("topbar").kids(vec![
            m("div").class("brand").kids(vec![
                Node::icon("gear-six"),
                m("b").text("title"),
                m("span").class("tag").child(lit("rust ⇄ wasm")),
            ]),
            m("nav").class("nav").child(
                m("a")
                    .for_each("r in routes")
                    .key("r.href")
                    .bind("href", "r.href")
                    .bind("class", "isActive(r.uri) ? 'on' : ''")
                    .on("click", "go($event)")
                    .kids(vec![Node::icon_dyn("r.icon"), m("span").text("r.label")]),
            ),
            m("span").class("topbar-right").child(
                Node::host("uiBadge", &[("icon", "lightning")])
                    .bind("tone", "$store.session.tone")
                    .bind("label", "$store.session.state")
                    .bind("pulse", "$store.session.building"),
            ),
        ]))
        .method("go", |_this, args| {
            mjs::router_link(&js::arg(&args, 0));
            JsValue::UNDEFINED
        })
        .method("isActive", |_this, args| {
            let want = js::arg_str(&args, 0);
            JsValue::from_bool(mjs::router_uri() == want)
        });
    mjs::component("navBar", &d.finish());
}

/// Reads and writes `$store.counter` — the value that has to survive a rebuild.
fn counter_card() {
    let mut d = Def::new().named("counterCard");
    d.prop_str("label", "store counter")
        .str("buttonLabel", demo::COUNTER_LABEL)
        .tree(m("div").class("card counter").kids(vec![
            m("div").class("card-title").kids(vec![
                Node::icon("arrows-clockwise"),
                m("span").text("label"),
            ]),
            m("div").class("counter-row").kids(vec![
                m("button")
                    .attr("type", "button")
                    .class("btn primary")
                    .on("click", "$store.counter.inc()")
                    .kids(vec![Node::icon("plus"), m("span").text("buttonLabel")]),
                m("button")
                    .attr("type", "button")
                    .class("btn")
                    .on("click", "$store.counter.inc(10)")
                    .child(lit("+10")),
                m("button")
                    .attr("type", "button")
                    .class("btn danger")
                    .on("click", "$store.counter.reset()")
                    .child(lit("reset")),
                m("output").class("counter-value").text("$store.counter.count"),
            ]),
            m("p").class("muted").kids(vec![
                lit("clicks "),
                m("b").text("$store.counter.clicks"),
                lit(" · last "),
                m("code").text("$store.counter.lastOp"),
                lit(" · this number lives in an m-js store on "),
                m("code").child(lit("window")),
                lit(", so the wasm rebuild keeps it"),
            ]),
        ]));
    mjs::component("counterCard", &d.finish());
}

/// The "try editing this" strip, fed by `demo::HINTS`.
fn hint_strip() {
    let mut d = Def::new().named("hintStrip");
    let hints = Array::new();
    for hint in demo::HINTS {
        hints.push(&JsValue::from_str(hint));
    }
    d.val("hints", &hints.into())
        .str("title", "what just happened")
        .tree(m("div").class("hints").kids(vec![
            m("div").class("hints-title").kids(vec![
                Node::icon("wrench"),
                m("span").text("title"),
            ]),
            m("ol").class("hints-list").child(
                m("li")
                    .for_each("(hint, i) in hints")
                    .key("i")
                    .child(m("code").text("hint")),
            ),
        ]));
    mjs::component("hintStrip", &d.finish());
}
