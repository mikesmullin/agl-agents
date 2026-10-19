//! Routes. A page is an `m()` tree handed to `Def::tree`, registered with
//! `Router.register(uri, title, fn(params))`.

use wasm_bindgen::prelude::*;

use rustex1_kit as kit;
use rustex1_kit::h::{lit, m, Node};
use rustex1_kit::mjs::Def;
use rustex1_kit::{hooks, js, mjs, session};

use crate::components;
use crate::demo;

pub fn register() {
    mjs::router_set_mode("path");
    mjs::router_register("/", "rustex1 · hot reload", |_| home());
    mjs::router_register("/storybook", "rustex1 · storybook", |_| storybook());
    mjs::router_register("/protocol", "rustex1 · protocol", |_| protocol());
    mjs::router_register("/counter/:n", "rustex1 · counter", |params| {
        counter_route(params)
    });
}

/// A titled card — the page-level building block.
fn card(icon: &str, title: &str, body: Vec<Node>) -> Node {
    m("article")
        .class("card")
        .kids(vec![m("div")
            .class("card-title")
            .kids(vec![Node::icon(icon), m("span").child(lit(title))])])
        .kids(body)
}

fn snippet(src: &str) -> Node {
    m("pre").class("src").child(lit(src))
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

fn home() -> JsValue {
    let mut d = Def::new().named("route /");
    d.str("headline", demo::HEADLINE)
        .str("subhead", demo::SUBHEAD)
        .str("budget", &format!("{}ms", demo::BUILD_BUDGET_MS as u64))
        .str("componentCount", &components::COUNT.to_string())
        .tree(m("div").class("shell").kids(vec![
            Node::host("navBar", &[]),
            m("main").class("page").kids(vec![
                // -- hero
                m("section").class("hero").kids(vec![
                    m("h1").text("headline"),
                    m("p").class("sub").text("subhead"),
                    m("div").class("hero-meta").kids(vec![
                        Node::host(
                            "uiBadge",
                            &[("tone", "info"), ("icon", "cube"), ("label", "rust backend")],
                        ),
                        Node::host(
                            "uiBadge",
                            &[
                                ("tone", "info"),
                                ("icon", "lightning"),
                                ("label", "rust → wasm"),
                            ],
                        ),
                        Node::host("uiBadge", &[("icon", "arrows-clockwise")])
                            .bind("tone", "$store.session.tone")
                            .bind("label", "$store.session.state")
                            .bind("pulse", "$store.session.building"),
                    ]),
                ]),
                // -- pipeline
                m("section").class("card").kids(vec![
                    m("div").class("card-title").kids(vec![
                        Node::icon("flow-arrow"),
                        m("span").child(lit("hot reload pipeline")),
                        m("span").class("grow"),
                        m("span").class("muted tiny").kids(vec![
                            lit("last rebuild over "),
                            m("span").text("budget"),
                            lit(" is amber"),
                        ]),
                    ]),
                    Node::host("pipelinePanel", &[]),
                    Node::host(
                        "gaugeBar",
                        &[
                            ("label", demo::GAUGE_LABEL),
                            (
                                "caption",
                                "width is one setAttribute per frame; everything else is a diff",
                            ),
                        ],
                    )
                    .bind("pct", "$store.meters.pctText"),
                    m("div").class("row").kids(vec![
                        m("button")
                            .attr("type", "button")
                            .class("btn primary")
                            .on("click", "rebuild()")
                            .bind("disabled", "$store.session.building")
                            .kids(vec![
                                Node::icon("hammer"),
                                m("span").child(lit("rebuild wasm (POST /api/rebuild)")),
                            ]),
                        m("button")
                            .attr("type", "button")
                            .class("btn")
                            .on("click", "ping()")
                            .kids(vec![
                                Node::icon("chat-circle"),
                                m("span").child(lit("say hello")),
                            ]),
                        m("button")
                            .attr("type", "button")
                            .class("btn danger")
                            .on("click", "restart()")
                            .kids(vec![Node::icon("power"), m("span").child(lit("restart server"))]),
                        m("span").class("muted tiny").kids(vec![
                            lit("fetches "),
                            m("b").text("$store.session.fetches"),
                            lit(" · swaps "),
                            m("b").text("$store.session.swaps"),
                            lit(" · last file "),
                            m("code").text("$store.session.lastFile"),
                        ]),
                    ]),
                    m("pre")
                        .class("errbox")
                        .show("$store.session.error")
                        .text("$store.session.error"),
                    // template lint findings (see lint.rs)
                    m("pre")
                        .class("errbox")
                        .show("$store.session.lintCount > 0")
                        .text("$store.session.lintText"),
                ]),
                // -- state that survives
                Node::host("counterCard", &[]),
                m("div").class("grid two").kids(vec![
                    Node::host("buildFeed", &[]),
                    card("bell", "toasts", vec![Node::host("toastStack", &[])]),
                ]),
                Node::host("hintStrip", &[]),
            ]),
            m("footer").class("foot").child(
                m("span").class("muted tiny").kids(vec![
                    lit("components "),
                    m("b").text("componentCount"),
                    lit(" · template lint "),
                    m("b").text("$store.session.lintRules"),
                    lit(" rules · copy in app/src/demo.rs"),
                ]),
            ),
        ]))
        .method("rebuild", |_this, _args| {
            kit::api::rebuild_now();
            JsValue::UNDEFINED
        })
        .method("restart", |_this, _args| {
            kit::api::restart_server();
            JsValue::UNDEFINED
        })
        .method("ping", |this, _args| {
            let n = js::get_num(&this, "pings") + 1.0;
            js::set_num(&this, "pings", n);
            session::toast(&format!("hello from rust #{n}"), "info");
            JsValue::UNDEFINED
        })
        .num("pings", 0.0);
    d.finish()
}

// ---------------------------------------------------------------------------
// Storybook — every component in the catalog, rendered from Rust
// ---------------------------------------------------------------------------

fn storybook() -> JsValue {
    let mut d = Def::new().named("route /storybook");
    d.str("title", "component storybook")
        .tree(m("div").class("shell").kids(vec![
            Node::host("navBar", &[]),
            m("main").class("page").kids(vec![
                m("section").class("hero").kids(vec![
                    m("h1").text("title"),
                    m("p").class("sub").child(lit(
                        "each widget is a Rust function in app/src/components.rs → M.component(name, def)",
                    )),
                ]),
                m("div").class("grid two").kids(vec![
                    card(
                        "badge",
                        "uiBadge",
                        vec![
                            m("div").class("row").kids(vec![
                                Node::host(
                                    "uiBadge",
                                    &[("tone", "ok"), ("label", "ok"), ("icon", "check-circle")],
                                ),
                                Node::host(
                                    "uiBadge",
                                    &[
                                        ("tone", "busy"),
                                        ("label", "busy"),
                                        ("icon", "circle-notch"),
                                        ("pulse", "true"),
                                    ],
                                ),
                                Node::host(
                                    "uiBadge",
                                    &[("tone", "warn"), ("label", "warn"), ("icon", "warning")],
                                ),
                                Node::host(
                                    "uiBadge",
                                    &[("tone", "err"), ("label", "err"), ("icon", "x-circle")],
                                ),
                                Node::host("uiBadge", &[("tone", "idle"), ("label", "idle")]),
                            ]),
                            snippet(
                                r#"<span x-component="uiBadge" tone="ok" label="ok" icon="check-circle"></span>"#,
                            ),
                        ],
                    ),
                    card(
                        "cursor-click",
                        "uiButton",
                        vec![
                            m("div").class("row").kids(vec![
                                Node::host(
                                    "uiButton",
                                    &[
                                        ("label", "primary"),
                                        ("variant", "primary"),
                                        ("icon", "play"),
                                    ],
                                ),
                                Node::host(
                                    "uiButton",
                                    &[("label", "ghost"), ("variant", "ghost"), ("icon", "ghost")],
                                ),
                                Node::host(
                                    "uiButton",
                                    &[
                                        ("label", "danger"),
                                        ("variant", "danger"),
                                        ("icon", "trash"),
                                    ],
                                ),
                                Node::host(
                                    "uiButton",
                                    &[("label", "disabled"), ("variant", "ghost"), ("disabled", "true")],
                                ),
                            ]),
                            snippet(
                                r#"<span x-component="uiButton" label="primary" variant="primary" icon="play"></span>"#,
                            ),
                        ],
                    ),
                    card(
                        "squares-four",
                        "uiStat",
                        vec![
                            m("div").class("row").kids(vec![
                                Node::host(
                                    "uiStat",
                                    &[
                                        ("label", "builds"),
                                        ("value", "42"),
                                        ("hint", "since you opened the page"),
                                    ],
                                ),
                                Node::host(
                                    "uiStat",
                                    &[
                                        ("label", "tone: warn"),
                                        ("value", "3.1s"),
                                        ("tone", "warn"),
                                        ("icon", "timer"),
                                    ],
                                ),
                                Node::host("uiStat", &[("label", "live"), ("icon", "activity")])
                                    .bind("value", "$store.meters.pctText"),
                            ]),
                            snippet(r#"<span x-component="uiStat" label="builds" value="42" hint="…"></span>"#),
                        ],
                    ),
                    card(
                        "activity",
                        "gaugeBar",
                        vec![
                            Node::host(
                                "gaugeBar",
                                &[
                                    ("label", "shared signal"),
                                    ("caption", "same data-gauge node the rAF loop writes"),
                                ],
                            ),
                            snippet("width is set by lifecycle::on_frame → one setAttribute per frame"),
                        ],
                    ),
                    card(
                        "plus-minus",
                        "counterCard",
                        vec![Node::host(
                            "counterCard",
                            &[("label", "per-instance host label")],
                        )],
                    ),
                    card(
                        "bell",
                        "toastStack",
                        vec![
                            m("div").class("row").kids(vec![
                                m("button")
                                    .attr("type", "button")
                                    .class("btn")
                                    .on("click", "toast('info')")
                                    .child(lit("info")),
                                m("button")
                                    .attr("type", "button")
                                    .class("btn")
                                    .on("click", "toast('ok')")
                                    .child(lit("ok")),
                                m("button")
                                    .attr("type", "button")
                                    .class("btn")
                                    .on("click", "toast('warn')")
                                    .child(lit("warn")),
                                m("button")
                                    .attr("type", "button")
                                    .class("btn")
                                    .on("click", "toast('err')")
                                    .child(lit("err")),
                                m("button")
                                    .attr("type", "button")
                                    .class("btn tiny")
                                    .on("click", "clear()")
                                    .child(lit("clear")),
                            ]),
                            Node::host("toastStack", &[]),
                        ],
                    ),
                    card(
                        "list-bullets",
                        "buildFeed",
                        vec![Node::host("buildFeed", &[("title", "watcher log")])],
                    ),
                    card("wrench", "hintStrip", vec![Node::host("hintStrip", &[])]),
                ]),
            ]),
        ]))
        .method("toast", |_this, args| {
            let tone = {
                let t = js::arg_str(&args, 0);
                if t.is_empty() {
                    "info".to_string()
                } else {
                    t
                }
            };
            session::toast(&format!("storybook {tone} toast"), &tone);
            JsValue::UNDEFINED
        })
        .method("clear", |_this, _args| {
            clear_toasts();
            JsValue::UNDEFINED
        });
    d.finish()
}

fn clear_toasts() {
    let t = kit::mjs::store_ref("toasts");
    js::call(&t, "clear", &[]);
}

// ---------------------------------------------------------------------------
// Protocol — the wire format, self-documented
// ---------------------------------------------------------------------------

fn protocol() -> JsValue {
    let mut d = Def::new().named("route /protocol");
    d.str(
        "hello",
        r#"{ "type": "connected", "bootId": "…", "server": "rustex1" }"#,
    )
    .str(
        "change",
        r#"{ "type": "change", "path": "/pkg/rustex1_app.js" }"#,
    )
    .str(
        "build",
        r#"{ "type": "build", "phase": "ok", "target": "wasm", "ms": 1840 }"#,
    )
    .str("lastMessage", "—")
    .tree(m("div").class("shell").kids(vec![
        Node::host("navBar", &[]),
        m("main").class("page").kids(vec![
            m("section").class("hero").kids(vec![
                m("h1").child(lit("one socket, two messages")),
                m("p").class("sub").child(lit(
                    "the client side is m-js's hot reload protocol; the server side is Rust",
                )),
            ]),
            m("div").class("grid two").kids(vec![
                card(
                    "plugs-connected",
                    "server → client",
                    vec![
                        m("pre").class("src").text("hello"),
                        m("pre").class("src").text("change"),
                        m("pre").class("src").text("build"),
                    ],
                ),
                card(
                    "arrows-left-right",
                    "what the client does",
                    vec![m("table").class("kv").kids(vec![
                        kv(".css", vec![lit("clone + cache-bust the <link>")]),
                        kv(
                            ".js / .mjs",
                            vec![lit("re-import, then "), code("window.__M_BOOT__(t)")],
                        ),
                        kv(
                            ".html",
                            vec![lit("full reload (the document shell changed)")],
                        ),
                        kv(
                            "bootId differs",
                            vec![lit("full reload — the server process was replaced")],
                        ),
                        kv(
                            "build",
                            vec![lit(
                                "progress line; a failed build flashes + logs (no cargo overlay — that is the terminal's)",
                            )],
                        ),
                    ])],
                ),
            ]),
            card(
                "terminal-window",
                "last ws message",
                vec![
                    m("pre").class("src").text("lastMessage"),
                    m("div").class("row").kids(vec![
                            m("button")
                                .attr("type", "button")
                                .class("btn")
                                .on("click", "refresh()")
                                .kids(vec![
                                    Node::icon("arrows-clockwise"),
                                    m("span").child(lit("read again")),
                                ]),
                            m("span").class("muted tiny").child(lit(
                                "window.__HMR__.lastMessage, read from Rust via the hooks bridge",
                            )),
                        ]),
                ],
            ),
        ]),
    ]))
    .method("refresh", |this, _args| {
        let msg = js::get_str(&hooks::stats(), "lastMessage");
        js::set_str(
            &this,
            "lastMessage",
            if msg.is_empty() { "—" } else { &msg },
        );
        JsValue::UNDEFINED
    });
    d.finish()
}

/// A row of the behaviour table: `<code>key</code> | description nodes`.
fn kv(key: &str, detail: Vec<Node>) -> Node {
    m("tr").kids(vec![m("td").child(code(key)), m("td").kids(detail)])
}

fn code(text: &str) -> Node {
    m("code").child(lit(text))
}

// ---------------------------------------------------------------------------

fn counter_route(params: JsValue) -> JsValue {
    let mut d = Def::new().named("route /counter/:n");
    d.val("params", &params)
        .tree(m("div").class("shell").kids(vec![
            Node::host("navBar", &[]),
            m("main").class("page").kids(vec![
                m("section").class("hero").kids(vec![
                    m("h1").child(lit("route params, from Rust")),
                    m("p").class("sub").child(lit(
                        "Router.register(\"/counter/:n\", …, (params) => component)",
                    )),
                ]),
                card(
                    "hash",
                    "matched params",
                    vec![m("pre").class("src").text("JSON.stringify(params)")],
                ),
                Node::host("counterCard", &[("label", "same store, different route")]),
            ]),
        ]));
    d.finish()
}
