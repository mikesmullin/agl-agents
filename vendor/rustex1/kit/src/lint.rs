//! Template linter — kills the *other* silent failure mode.
//!
//! `h.rs` makes mis-closed tags unrepresentable. This covers what markup
//! discipline cannot: an expression or a name inside a directive that is simply
//! wrong. m-js compiles every `x-text` / `@click` / `:class` with
//! `new Function('$scope','$magics', 'with($magics){with($scope){return (EXPR)}}'`)
//! and **swallows both failure paths**:
//!
//! ```text
//!   x-text="$store.sesion.count"     → compiles, throws at eval, returns undefined → blank
//!   x-text="count +"                 → new Function throws, cached as null       → blank
//!   x-component="uiButon"            → console.warn at render                     → nothing
//!   x-for="items"                    → parseForExpression returns null            → no rows
//! ```
//!
//! So at boot we walk the recorded `m()` trees (structure, not markup — no
//! re-parsing) and check every directive: known name, compiles, `$store.<name>`
//! registered, `x-component` registered, `x-for` shaped right. Findings go to
//! the console, into `$store.session.lint*`, and into the same red overlay the
//! compile errors use.

use wasm_bindgen::prelude::*;

use crate::h;
use crate::hooks;
use crate::js;
use crate::mjs;

#[derive(Clone)]
pub struct Issue {
    pub source: String,
    pub tag: String,
    pub attr: String,
    pub kind: &'static str,
    pub detail: String,
}

impl Issue {
    pub fn render(&self) -> String {
        format!(
            "{} · <{}> {} [{}] {}",
            self.source, self.tag, self.attr, self.kind, self.detail
        )
    }
}

/// Directive grammar m-js understands (x- and m- prefixes).
const DIRECTIVES: &[&str] = &[
    "data",
    "init",
    "for",
    "if",
    "model",
    "bind",
    "text",
    "html",
    "show",
    "transition",
    "on",
    "effect",
    "cloak",
    "ignore",
    "ref",
    "mount",
    "component",
];

/// Directives whose value is a *statement* rather than an expression.
const ACTIONS: &[&str] = &["on", "init", "effect"];

fn directive_of(name: &str) -> Option<&str> {
    if let Some(d) = name.strip_prefix("x-") {
        return Some(d);
    }
    if let Some(d) = name.strip_prefix("m-") {
        return Some(d);
    }
    None
}

fn is_expression_attr(name: &str) -> bool {
    if let Some(d) = directive_of(name) {
        return !matches!(
            d,
            "cloak" | "ignore" | "ref" | "component" | "transition" | "for"
        );
    }
    name.starts_with(':')
}

// ---------------------------------------------------------------------------

pub fn check_all() -> Vec<Issue> {
    let mut out = Vec::new();

    for rec in h::recorded() {
        rec.node.walk(&mut |node, _depth| {
            if node.is_text() {
                return;
            }
            let tag = node.tag();
            for (name, value) in node.attrs() {
                check_attr(&rec.source, tag, name, value, &mut out);
            }
            // A `:key` without an `x-for` is silently meaningless: static
            // siblings are keyed positionally by the diff.
            if node.attrs().iter().any(|(k, _)| k == ":key")
                && !node.attrs().iter().any(|(k, _)| k == "x-for" || k == "m-for")
            {
                out.push(Issue {
                    source: rec.source.clone(),
                    tag: tag.to_string(),
                    attr: ":key".into(),
                    kind: "key",
                    detail: ":key only carries identity on an x-for row; on a static element the diff keys by position".into(),
                });
            }
        });
    }

    out
}

fn check_attr(source: &str, tag: &str, name: &str, value: &str, out: &mut Vec<Issue>) {
    // 1 · unknown directive spelling
    if let Some(d) = directive_of(name) {
        if !DIRECTIVES.contains(&d) {
            out.push(Issue {
                source: source.to_string(),
                tag: tag.to_string(),
                attr: name.to_string(),
                kind: "directive",
                detail: format!(
                    "no such directive `x-{d}` — m-js will emit it as a plain attribute"
                ),
            });
            return;
        }
        if d == "component" && !mjs::has_component(value) {
            out.push(Issue {
                source: source.to_string(),
                tag: tag.to_string(),
                attr: name.to_string(),
                kind: "component",
                detail: format!("no widget named `{value}` is registered"),
            });
        }
        if d == "for" {
            check_for(source, tag, value, out);
        }
    }

    let expression_like =
        name.starts_with('@') || name.starts_with(':') || is_expression_attr(name);
    if !expression_like {
        return;
    }

    // 2 · empty directive
    if value.trim().is_empty() {
        out.push(Issue {
            source: source.to_string(),
            tag: tag.to_string(),
            attr: name.to_string(),
            kind: "empty",
            detail: "directive value is empty".into(),
        });
        return;
    }

    // 3 · store names must exist
    for store in stores_referenced(value) {
        if !mjs::has_store(&store) {
            out.push(Issue {
                source: source.to_string(),
                tag: tag.to_string(),
                attr: name.to_string(),
                kind: "store",
                detail: format!(
                    "`$store.{store}` is not registered (typo?) — m-js would render blank"
                ),
            });
        }
    }

    // 4 · it has to compile
    let action = directive_of(name)
        .map(|d| ACTIONS.contains(&d))
        .unwrap_or(name.starts_with('@'));
    if let Some(err) = compile_error(value, action) {
        out.push(Issue {
            source: source.to_string(),
            tag: tag.to_string(),
            attr: name.to_string(),
            kind: "syntax",
            detail: err,
        });
    }
}

/// `x-for` accepts `item in list`, `(item, i) in list`, `[a, b] in list`, …
fn check_for(source: &str, tag: &str, value: &str, out: &mut Vec<Issue>) {
    let v = value.trim();
    let has_in = split_top_level(v, " in ").is_some() || split_top_level(v, " of ").is_some();
    if !has_in {
        out.push(Issue {
            source: source.to_string(),
            tag: tag.to_string(),
            attr: "x-for".into(),
            kind: "for",
            detail: format!("`{v}` is not `<item> in <list>` — the loop renders nothing"),
        });
        return;
    }
    let (_, list) = split_top_level(v, " in ")
        .or_else(|| split_top_level(v, " of "))
        .unwrap();
    let list = list.trim();
    if list.is_empty() {
        out.push(Issue {
            source: source.to_string(),
            tag: tag.to_string(),
            attr: "x-for".into(),
            kind: "for",
            detail: "no iterable after `in`".into(),
        });
    }
    for store in stores_referenced(list) {
        if !mjs::has_store(&store) {
            out.push(Issue {
                source: source.to_string(),
                tag: tag.to_string(),
                attr: "x-for".into(),
                kind: "store",
                detail: format!("`$store.{store}` is not registered"),
            });
        }
    }
}

/// Split on `sep` but only when outside (), [], {}, quotes.
fn split_top_level<'a>(s: &'a str, sep: &str) -> Option<(&'a str, &'a str)> {
    let bytes = s.as_bytes();
    let needle = sep.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q && bytes.get(i.wrapping_sub(1)) != Some(&b'\\') {
                    quote = None;
                }
            }
            None => {
                match b {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth -= 1,
                    b'\'' | b'"' | b'`' => quote = Some(b),
                    _ => {}
                }
                if depth == 0 && b == needle[0] && bytes[i..].starts_with(needle) {
                    return Some((&s[..i], &s[i + needle.len()..]));
                }
            }
        }
        i += 1;
    }
    None
}

/// Every `$store.<name>` referenced in an expression.
fn stores_referenced(expr: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if &bytes[i..i + 7] == b"$store." {
            let mut j = i + 7;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'$')
            {
                j += 1;
            }
            let name = &expr[i + 7..j];
            if !name.is_empty() && !found.iter().any(|f| f == name) {
                found.push(name.to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

/// Compile the way m-js compiles — and return the message it would swallow.
fn compile_error(expr: &str, action: bool) -> Option<String> {
    let body = if action {
        format!("with ($magics) {{ with ($scope) {{ {expr} }} }}")
    } else {
        format!("with ($magics) {{ with ($scope) {{ return ({expr}); }} }}")
    };

    let ctor = js::get(&js::global(), "Function");
    let ctor = ctor.dyn_into::<js_sys::Function>().ok()?;
    let args = js_sys::Array::of3(
        &JsValue::from_str("$scope"),
        &JsValue::from_str("$magics"),
        &JsValue::from_str(&body),
    );
    match ctor.apply(&JsValue::NULL, &args) {
        Ok(_) => None,
        Err(e) => Some(js::describe(&e)),
    }
}

// ---------------------------------------------------------------------------

thread_local! {
    static LAST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many findings the previous [`run`] produced.
pub fn last_count() -> usize {
    LAST.with(|c| c.get())
}

/// Run the checks, report them everywhere a developer will look.
pub fn run() -> Vec<Issue> {
    let issues = check_all();

    // Prove the checker is not vacuous: a synthetic tree containing every
    // defect kind must produce every rule.
    let (fired, missed) = self_test();

    if issues.is_empty() {
        hooks::hide_overlay();
        LAST.with(|c| c.set(0));
        crate::session::write_num("session", "lintCount", 0.0);
        crate::session::write_str(
            "session",
            "lintText",
            &if missed.is_empty() {
                String::new()
            } else {
                format!(
                    "linter self-test incomplete: {} not firing",
                    missed.join(", ")
                )
            },
        );
        js::log(&format!(
            "[rustex1] template lint: clean (self-test {}/{} rules)",
            fired.len(),
            RULES.len()
        ));
        crate::session::write_str(
            "session",
            "lintRules",
            &format!("{}/{}", fired.len(), RULES.len()),
        );
        return issues;
    }

    let text: String = issues
        .iter()
        .map(|i| i.render())
        .collect::<Vec<_>>()
        .join("\n");

    for i in &issues {
        js::error(&format!("[rustex1 lint] {}", i.render()));
    }
    LAST.with(|c| c.set(issues.len()));
    crate::session::write_num("session", "lintCount", issues.len() as f64);
    crate::session::write_str("session", "lintText", &text);
    hooks::show_overlay(
        &format!("{} template issue(s)\n\n{}", issues.len(), text),
        "template lint",
    );
    issues
}

const RULES: &[&str] = &[
    "directive",
    "syntax",
    "store",
    "component",
    "for",
    "key",
    "empty",
];

/// A tree containing exactly one defect per rule. If any rule does not fire,
/// the linter is lying to us.
fn self_test() -> (Vec<String>, Vec<String>) {
    use crate::h::m;

    let probe = m("div")
        .attr("x-tex", "label") // unknown directive
        .text("count +") // syntax error
        .inner_html("$store.nope.field") // unregistered store
        .attr("x-component", "nopeNotAWidget") // unregistered widget
        .for_each("items") // malformed x-for
        .effect("") // empty value
        .child(m("span").key("k")); // :key with no x-for

    let mut issues = Vec::new();
    probe.walk(&mut |node, _depth| {
        if node.is_text() {
            return;
        }
        for (name, value) in node.attrs() {
            check_attr("self-test", node.tag(), name, value, &mut issues);
        }
        if node.attrs().iter().any(|(k, _)| k == ":key")
            && !node.attrs().iter().any(|(k, _)| k == "x-for")
        {
            issues.push(Issue {
                source: "self-test".into(),
                tag: node.tag().to_string(),
                attr: ":key".into(),
                kind: "key",
                detail: "probe".into(),
            });
        }
    });

    let fired: Vec<String> = RULES
        .iter()
        .filter(|r| issues.iter().any(|i| i.kind == **r))
        .map(|s| s.to_string())
        .collect();
    let missed: Vec<String> = RULES
        .iter()
        .filter(|r| !fired.iter().any(|f| f == *r))
        .map(|s| s.to_string())
        .collect();
    (fired, missed)
}
