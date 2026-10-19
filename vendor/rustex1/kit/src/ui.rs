//! The kit's widget catalog — reusable m-js components defined in Rust and
//! registered on every boot, so an app gets a styled UI layer for free.
//!
//! | name | props |
//! |---|---|
//! | `uiBadge` | `tone, label, icon, pulse` |
//! | `uiButton` | `label, variant, icon, disabled` |
//! | `uiStat` | `label, value, hint, tone, icon` |
//! | `gaugeBar` | `label, gauge, caption` |
//! | `toastStack` | `empty` |
//! | `pipelinePanel` | — (reads `$store.session`) |
//! | `buildFeed` | `title` |
//!
//! Templates are `m()` trees, never markup strings. Usage from a page template:
//!
//! ```ignore
//! Node::host("uiBadge", &[("tone", "ok"), ("label", "live"), ("icon", "check-circle")])
//! ```
//!
//! To swap in your own version of any of these, register the same name in
//! `App::register_components` — the app runs after the kit.

use wasm_bindgen::prelude::*;

use crate::h::{lit, m, Node};
use crate::mjs::{self, Def};

pub fn register() {
    ui_badge();
    ui_button();
    ui_stat();
    mjs::component("gaugeBar", &gauge_bar("pipeline"));
    toast_stack();
    pipeline_panel();
    build_feed();
    crate::ui_tooltip::register();
    crate::ui_reltime::register();
    crate::ui_form::register();
}

/// How many widgets the kit provides (including the seven form fields) —
/// apps print this in their footer.
pub const COUNT: usize = 9 + crate::ui_form::COUNT;

// ---------------------------------------------------------------------------
// atoms
// ---------------------------------------------------------------------------

fn ui_badge() {
    let mut d = Def::new().named("uiBadge");
    d.prop_str("tone", "idle")
        .prop_str("label", "—")
        .prop_str("icon", "")
        .prop_bool("pulse", false)
        .tree(m("span").class("badge").bind("class", "tone").kids(vec![
            m("i").show("icon").bind("class", "'ph ph-' + icon"),
            m("i").show("pulse").class("pulse").attr("aria-hidden", "true"),
            m("span").text("label"),
        ]));
    mjs::component("uiBadge", &d.finish());
}

fn ui_button() {
    let mut d = Def::new().named("uiButton");
    d.prop_str("label", "Button")
        .prop_str("variant", "ghost")
        .prop_str("icon", "")
        .prop_bool("disabled", false)
        .tree(
            m("button")
                .attr("type", "button")
                .class("btn")
                .bind("class", "variant")
                .bind("disabled", "disabled")
                .kids(vec![
                    m("i").show("icon").bind("class", "'ph ph-' + icon"),
                    m("span").text("label"),
                ]),
        );
    mjs::component("uiButton", &d.finish());
}

fn ui_stat() {
    let mut d = Def::new().named("uiStat");
    d.prop_str("label", "stat")
        .prop_str("value", "—")
        .prop_str("hint", "")
        .prop_str("tone", "idle")
        .prop_str("icon", "chart-bar")
        .tree(m("div").class("stat").bind("class", "tone").kids(vec![
            m("div").class("stat-label").kids(vec![
                Node::icon_dyn("icon"),
                m("span").text("label"),
            ]),
            m("div").class("stat-value").text("value"),
            m("div").class("stat-hint").show("hint").text("hint"),
        ]));
    mjs::component("uiStat", &d.finish());
}

// ---------------------------------------------------------------------------
// the fast one
// ---------------------------------------------------------------------------

/// The frame-rate widget, built the way m-js wants fast things built: the
/// structure is rendered once from a template, and the app's `requestAnimationFrame`
/// loop writes **one attribute** — the fill rect's `width` — and nothing else.
///
/// `id` becomes `data-gauge="<id>"`, which is the handshake the app's frame loop
/// queries on. Give each gauge its own id and register more than one:
///
/// ```ignore
/// rt.component("cpuGauge", &rustex1_kit::ui::gauge_bar("cpu"));
/// ```
pub fn gauge_bar(id: &str) -> JsValue {
    let mut d = Def::new().named("gaugeBar");
    d.prop_str("label", "load")
        .prop_str("gauge", id)
        .prop_str("caption", "")
        // The percentage label is app data, not widget data: bind it from the
        // host (`Node::host("gaugeBar", …).bind("pct", "$store.meters.pctText")`)
        // so the widget never reaches into an app store.
        .prop_str("pct", "—")
        .tree(m("div").class("gauge").kids(vec![
            m("div").class("gauge-head").kids(vec![
                m("span").class("gauge-label").text("label"),
                m("span").class("gauge-pct").text("pct"),
            ]),
            // SVG through the serializer: the parser is the browser's, so foreign
            // content (viewBox casing et al.) behaves exactly as in a literal
            // template.
            m("svg")
                .at(&[
                    ("class", "gauge-svg"),
                    ("viewBox", "0 0 240 10"),
                    ("preserveAspectRatio", "none"),
                    ("width", "240"),
                    ("height", "10"),
                    ("role", "img"),
                ])
                .bind("aria-label", "label")
                .kids(vec![
                    m("rect").at(&[
                        ("class", "gauge-track"),
                        ("x", "0"),
                        ("y", "0"),
                        ("width", "240"),
                        ("height", "10"),
                        ("rx", "5"),
                    ]),
                    m("rect").at(&[
                        ("class", "gauge-fill"),
                        ("data-gauge", id),
                        ("x", "0"),
                        ("y", "0"),
                        ("width", "0"),
                        ("height", "10"),
                        ("rx", "5"),
                    ]),
                    m("rect").at(&[
                        ("class", "gauge-budget"),
                        ("x", "0"),
                        ("y", "0"),
                        ("width", "1"),
                        ("height", "10"),
                    ]),
                ]),
            m("div").class("gauge-caption").show("caption").text("caption"),
        ]));
    d.finish()
}

// ---------------------------------------------------------------------------
// toasts
// ---------------------------------------------------------------------------

fn toast_stack() {
    let mut d = Def::new().named("toastStack");
    d.prop_str("empty", "no toasts")
        .tree(m("div").class("toasts").kids(vec![
            // `&& empty`: a host that clears the label wants no empty state at
            // all, not an empty paragraph. This matters most for a stack parked
            // `position: fixed` in a corner, where "no toasts" would sit there
            // permanently advertising that nothing happened — an empty-state
            // string is opt-in, and the corner host opts out.
            m("p")
                .class("muted")
                .show("$store.toasts.items.length === 0 && empty")
                .text("empty"),
            m("div")
                .class("toast")
                .for_each("t in $store.toasts.items")
                .key("t.id")
                .bind("class", "t.tone")
                .kids(vec![
                    Node::icon("lightbulb"),
                    m("span").class("toast-text").text("t.text"),
                    m("button")
                        .at(&[("type", "button"), ("class", "toast-x")])
                        .on("click", "$store.toasts.dismiss(t.id)")
                        .child(lit("×")),
                ]),
        ]));
    mjs::component("toastStack", &d.finish());
}

// ---------------------------------------------------------------------------
// dev-server chrome: any app built on this stack gets the pipeline for free
// ---------------------------------------------------------------------------

fn pipeline_panel() {
    let mut d = Def::new().named("pipelinePanel");
    d.tree(m("div").class("grid stats").kids(vec![
        Node::host("uiStat", &[("label", "server uptime"), ("icon", "timer")])
            .bind("value", "Math.round($store.session.api.uptimeS || 0) + 's'")
            .bind("hint", "'boot ' + ($store.session.api.bootId || '—')"),
        Node::host(
            "uiStat",
            &[
                ("label", "http requests"),
                ("icon", "globe-hemisphere-west"),
                ("hint", "served by rust, no framework"),
            ],
        )
        .bind("value", "String($store.session.api.requests || 0)"),
        Node::host(
            "uiStat",
            &[("label", "ws clients"), ("icon", "plugs-connected"), ("hint", "/__m_hmr")],
        )
        .bind("value", "String($store.session.api.clients || 0)"),
        Node::host(
            "uiStat",
            &[
                ("label", "last rebuild"),
                ("icon", "hammer"),
                ("hint", "cargo + wasm-bindgen"),
            ],
        )
        .bind("value", "$store.session.buildLabel")
        .bind("tone", "$store.session.buildTone"),
    ]));
    mjs::component("pipelinePanel", &d.finish());
}

fn build_feed() {
    let mut d = Def::new().named("buildFeed");
    d.prop_str("title", "build log")
        .tree(m("div").class("card feed").kids(vec![
            m("div").class("card-title").kids(vec![
                Node::icon("list-bullets"),
                m("span").text("title"),
                m("button")
                    .at(&[("type", "button"), ("class", "btn tiny")])
                    .on("click", "refresh()")
                    .child(lit("refresh")),
            ]),
            m("p")
                .class("muted")
                .show("$store.session.builds.length === 0")
                .child(lit("nothing yet — edit a .rs file")),
            m("ul").class("feed-list").child(
                m("li")
                    .for_each("b in $store.session.builds")
                    .key("b.seq")
                    .bind("class", "b.ok ? 'ok' : 'err'")
                    .kids(vec![
                        m("span").class("feed-seq").text("'#' + b.seq"),
                        m("span").class("feed-target").text("b.target"),
                        m("span").class("feed-ms").text("b.ms + 'ms'"),
                        m("span").class("feed-trigger").text("(b.trigger || []).join(' ')"),
                        m("span").class("feed-state").text("b.ok ? 'ok' : 'failed'"),
                    ]),
            ),
            m("pre").class("build-error").show("error").text("error"),
        ]))
        .method("refresh", |_this, _args| {
            crate::api::fetch_builds();
            JsValue::UNDEFINED
        });
    mjs::component("buildFeed", &d.finish());
}
