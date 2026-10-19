//! `m()` — hyperscript for m-js templates, written in Rust.
//!
//! m-js v3 has no hyperscript: templates are HTML strings, and `m` in
//! `import m from 'm-js'` is just an alias for the `M` namespace. The strings
//! are handed to the browser's own parser (`<template>.innerHTML`), which is
//! fast and forgiving — **too** forgiving:
//!
//! ```text
//!   <div><span x-text="one"></div><p x-text="two"></p></div>
//!   → [ div > span, p ]        // no error, no warning; the span escaped the div
//! ```
//!
//! So this module is the serializer side of the templates: you describe the
//! tree with nested calls and it *emits* the markup m-js wants. A mismatched
//! tag stops being expressible — nesting is structural, not textual — while
//! every directive (`x-*`, `@`, `:`) keeps working exactly as documented,
//! because what comes out is still a template string for the same parser.
//!
//! ```ignore
//! m("i").at(&[("x-show", "icon")]).bind("class", "'ph ph-' + icon")
//! // <i x-show="icon" :class="'ph ph-' + icon"></i>
//! ```
//!
//! Directives have typed shorthands (`.text`, `.show`, `.on`, `.for_each`,
//! `.key`, `.widget`…) so the common ones do not need their `x-` spelling at
//! all. Emission is pretty-printed, and the newlines between siblings are
//! load-bearing: they are the whitespace text nodes that separate inline
//! elements, which is what hand-written multi-line templates produce too.

use std::cell::RefCell;
use std::fmt::Write as _;

/// Elements that never take a closing tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

#[derive(Clone)]
pub struct Node {
    tag: String,
    /// A text node: `tag` holds the literal text and is emitted escaped.
    text: bool,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

// ---------------------------------------------------------------------------
// registry — every tree handed to a component or page is remembered so the
// linter (`lint.rs`) can walk structure instead of re-parsing markup
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Recorded {
    pub source: String,
    pub node: Node,
}

thread_local! {
    static RECORD: RefCell<Vec<Recorded>> = const { RefCell::new(Vec::new()) };
}

pub fn record(source: &str, node: &Node) {
    RECORD.with(|r| {
        r.borrow_mut().push(Recorded {
            source: source.to_string(),
            node: node.clone(),
        })
    });
}

pub fn recorded() -> Vec<Recorded> {
    RECORD.with(|r| r.borrow().clone())
}

impl Node {
    /// Depth-first walk of this subtree, including self.
    pub fn walk(&self, visit: &mut impl FnMut(&Node, usize)) {
        self.walk_at(0, visit);
    }

    fn walk_at(&self, depth: usize, visit: &mut impl FnMut(&Node, usize)) {
        visit(self, depth);
        for c in &self.children {
            c.walk_at(depth + 1, visit);
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn attrs(&self) -> &[(String, String)] {
        &self.attrs
    }

    pub fn is_text(&self) -> bool {
        self.text
    }
}

/// Start an element.
pub fn m(tag: &str) -> Node {
    Node {
        tag: tag.to_string(),
        text: false,
        attrs: Vec::new(),
        children: Vec::new(),
    }
}

/// Literal text child, escaped on emit.
pub fn lit(text: &str) -> Node {
    Node {
        tag: text.to_string(),
        text: true,
        attrs: Vec::new(),
        children: Vec::new(),
    }
}

/// Several nodes with no wrapper — a template root.
pub fn frag(nodes: Vec<Node>) -> String {
    let mut out = String::new();
    for (i, n) in nodes.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        n.emit(&mut out, 0);
    }
    out
}

impl Node {
    // -- attributes ---------------------------------------------------------

    /// Literal attribute pairs — the `m('i', { x-show: 'icon' })` shape.
    pub fn at(mut self, list: &[(&str, &str)]) -> Self {
        for (k, v) in list {
            self.put(k, v);
        }
        self
    }

    pub fn attr(mut self, name: &str, value: &str) -> Self {
        self.put(name, value);
        self
    }

    /// Set an attribute, replacing one already set under the same name (later
    /// call wins). Keeps a chained builder from emitting duplicate
    /// attributes, which HTML resolves by silently keeping only the first.
    fn put(&mut self, name: &str, value: &str) {
        if let Some(existing) = self.attrs.iter_mut().find(|(k, _)| k == name) {
            existing.1 = value.to_string();
        } else {
            self.attrs.push((name.to_string(), value.to_string()));
        }
    }

    // -- directives ---------------------------------------------------------

    /// `:name="expr"` — distinct from `attr("name", …)`, so a static
    /// `class="badge"` and a bound `:class="tone"` coexist as m-js expects.
    pub fn bind(mut self, name: &str, expr: &str) -> Self {
        self.put(&format!(":{name}"), expr);
        self
    }

    /// `@event="expr"`
    pub fn on(mut self, event: &str, expr: &str) -> Self {
        self.put(&format!("@{event}"), expr);
        self
    }

    /// `x-text="expr"`
    pub fn text(self, expr: &str) -> Self {
        self.attr("x-text", expr)
    }

    /// `x-html="expr"`
    pub fn inner_html(self, expr: &str) -> Self {
        self.attr("x-html", expr)
    }

    /// `x-show="expr"`
    pub fn show(self, expr: &str) -> Self {
        self.attr("x-show", expr)
    }

    /// `x-if="expr"`
    pub fn iff(self, expr: &str) -> Self {
        self.attr("x-if", expr)
    }

    /// `x-for="item in items"` — pair with [Node::key].
    pub fn for_each(self, expr: &str) -> Self {
        self.attr("x-for", expr)
    }

    /// `:key="expr"` — diff identity, honoured on `x-for` rows.
    pub fn key(self, expr: &str) -> Self {
        self.bind("key", expr)
    }

    /// `x-model="expr"`
    pub fn model(self, expr: &str) -> Self {
        self.attr("x-model", expr)
    }

    /// `x-ref="name"`
    pub fn kref(self, name: &str) -> Self {
        self.attr("x-ref", name)
    }

    /// `x-init="expr"`
    pub fn init(self, expr: &str) -> Self {
        self.attr("x-init", expr)
    }

    /// `x-effect="expr"`
    pub fn effect(self, expr: &str) -> Self {
        self.attr("x-effect", expr)
    }

    /// `x-component="name"` — invoke a widget from the registry.
    pub fn widget(self, name: &str) -> Self {
        self.attr("x-component", name)
    }

    /// `x-ignore` — leave this subtree to the browser, m-js will not touch it.
    pub fn ignore(self) -> Self {
        self.attr("x-ignore", "")
    }

    // -- sugar --------------------------------------------------------------

    pub fn class(self, c: &str) -> Self {
        self.attr("class", c)
    }

    /// Static phosphor icon: `<i class="ph ph-check-circle"></i>`
    pub fn icon(name: &str) -> Self {
        m("i").class(&format!("ph ph-{name}"))
    }

    /// Dynamic phosphor icon: `<i :class="'ph ph-' + expr"></i>`
    pub fn icon_dyn(expr: &str) -> Self {
        m("i").bind("class", &format!("'ph ph-' + {expr}"))
    }

    /// A `<span x-component="name">` host carrying literal attrs.
    pub fn host(name: &str, attrs: &[(&str, &str)]) -> Self {
        m("span").widget(name).at(attrs)
    }

    /// Like [Node::host] but with `:bound` expression attrs.
    pub fn host_bound(name: &str, attrs: &[(&str, &str)]) -> Self {
        let mut node = m("span").widget(name);
        for (k, v) in attrs {
            node = node.bind(k, v);
        }
        node
    }

    // -- children -----------------------------------------------------------

    pub fn child(mut self, node: Node) -> Self {
        self.children.push(node);
        self
    }

    pub fn kids(mut self, nodes: Vec<Node>) -> Self {
        self.children.extend(nodes);
        self
    }

    // -- output -------------------------------------------------------------

    pub fn into_html(self) -> String {
        let mut out = String::new();
        self.emit(&mut out, 0);
        out
    }

    fn emit(&self, out: &mut String, depth: usize) {
        if self.text {
            out.push_str(&escape_text(&self.tag));
            return;
        }

        let pad = "  ".repeat(depth);
        if depth > 0 {
            out.push_str(&pad);
        }
        let _ = write!(out, "<{}", self.tag);
        for (k, v) in &self.attrs {
            if v.is_empty() && !k.starts_with('@') && !k.starts_with(':') && !k.starts_with("x-") {
                let _ = write!(out, " {k}");
            } else {
                let _ = write!(out, " {k}=\"{}\"", escape_attr(v));
            }
        }

        if VOID.contains(&self.tag.as_str()) {
            out.push('>');
            return;
        }
        out.push('>');

        if self.children.is_empty() {
            let _ = write!(out, "</{}>", self.tag);
            return;
        }

        // Elements whose children are all text stay on one line; mixed trees
        // are block-per-child with newlines, which is what hand-written
        // multi-line templates produce (and that whitespace is visible).
        let only_text = self.children.iter().all(|c| c.text);
        if only_text {
            for c in &self.children {
                c.emit(out, 0);
            }
        } else {
            out.push('\n');
            for c in &self.children {
                c.emit(out, depth + 1);
                out.push('\n');
            }
            if depth > 0 {
                out.push_str(&pad);
            }
        }
        let _ = write!(out, "</{}>", self.tag);
    }
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.clone_html())
    }
}

impl Node {
    fn clone_html(&self) -> String {
        let mut out = String::new();
        self.emit(&mut out, 0);
        out
    }
}

fn escape_attr(v: &str) -> String {
    // The browser's parser decodes these back before m-js compiles the
    // expression, so `'ph ph-' + icon` survives a round trip untouched.
    v.replace('&', "&amp;").replace('"', "&quot;")
}

fn escape_text(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
