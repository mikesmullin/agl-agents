//! Form field widgets — each one a reusable m-js component, so an app's form is
//! a list of `Node::host(...)` calls rather than hand-built inputs.
//!
//! They exist for one metric: **the fewest clicks and keystrokes to a submitted
//! task.** Each widget offers the shortest honest path to a valid value:
//!
//! | widget | replaces | why it is faster |
//! |---|---|---|
//! | [`TEXT`] `uiText` | `<input>` + label | Enter submits when `submitOnEnter`, so a bar is "type, Enter" |
//! | [`AREA`] `uiArea` | `<textarea>` | plain Enter = newline (as it should be), ⌘/Ctrl+Enter submits |
//! | [`NUMBER`] `uiNumber` | `<input type=number>` | − / + buttons: one click instead of typing a digit |
//! | [`DATE`] `uiDate` | `<input type=date>` | `today` `+1w` `+1mo` `clear` chips: one click, no picker, no typing |
//! | [`CHOICE`] `uiChoice` | `<select>` | segmented control: one click instead of open-then-choose; ←/→/↑/↓ also work |
//! | [`CHIPS`] `uiChips` | a comma-separated box | Enter/Space commits a token, Backspace pops the last, × removes any |
//! | [`TOGGLE`] `uiToggle` | a checkbox | one click on a target the size of its row |
//!
//! ## The value contract
//!
//! A widget is handed the **store object** and the **field name**, and reads and
//! writes that object directly:
//!
//! ```text
//!   host    <span x-component="uiText" name="summary" :model="$store.draft" label="Summary"></span>
//!   widget  reads  model[name]        (a computed `value`, so it renders current)
//!           writes model[name] = new  (→ m-js schedules the redraw)
//!           and also dispatches a bubbling `field` event { name, value }
//! ```
//!
//! The store write is primary: one property write on a reactive proxy is what
//! drives the UI, and a host needs no listener wiring. The `field` event is
//! published as well, so containers that prefer the documented m-js idiom —
//! `@field="onField($event)"` for a dirty flag, validation or autosave — get it
//! too. `tasks1/web/probe.html` is the live check that both still work.
//!
//! Enter/⌘Enter can reach the host either way: pass the page's method as a prop
//! (`:on-submit="submit"`, which the widget calls), or bind Enter on the host's
//! own container and let it bubble out of the widget:
//!
//! ```ignore
//! m("div").class("bar-row").on("keydown.enter", "submit()")
//! ```
//!
//! Both work. uiChips uses `.stop` on Enter/Space for that reason — committing a
//! token must not also submit the form.
//!
//! ## Three constraints that cost real debugging time
//!
//! * **Attribute names are lowercased by the HTML parser.** A prop declared
//!   `submitOnEnter` and set from markup as `submitOnEnter=""` arrives as
//!   `submitonenter`. Set and read props in kebab-case (`submit-on-enter`,
//!   `:on-submit`), which is idiomatic HTML anyway.
//! * **`$el` is the widget's rendered root**, not the host element. Host
//!   attributes become *props on the scope* — so `name` must be read from
//!   `this`, never from `$el`, or every event carries `name: ""`.
//! * **`js::cb` already passes `this`.** A method's closure signature is
//!   `(scope, callArgs)` where `callArgs[0]` is the template's first argument —
//!   indexing as if `this` occupied slot 0 silently reads `undefined`.
//! Widgets never name an app's store: `model` is whatever object the host passes,
//! and `name` is the key within it.

use js_sys::{Array, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::h::{lit, m, Node};
use crate::hooks;
use crate::js;
use crate::mjs::{self, Def};

pub const TEXT: &str = "uiText";
pub const AREA: &str = "uiArea";
pub const NUMBER: &str = "uiNumber";
pub const DATE: &str = "uiDate";
pub const CHOICE: &str = "uiChoice";
pub const CHIPS: &str = "uiChips";
pub const TOGGLE: &str = "uiToggle";

pub const COUNT: usize = 7;

pub fn register() {
    ui_text();
    ui_area();
    ui_number();
    ui_date();
    ui_choice();
    ui_chips();
    ui_toggle();
}

// ---------------------------------------------------------------------------
// the value contract
// ---------------------------------------------------------------------------

pub const FIELD_EVENT: &str = "field";

/// Publish a new value, two ways at once:
///
/// 1. **the store write** — `model[name] = value`. This is what drives the UI: a
///    property write on a reactive m-js proxy schedules the redraw, and a host
///    needs no listener wiring at all.
/// 2. **the m-js idiom** — a bubbling `field` event carrying `{name, value}`, so
///    a host container can also `@field="onField($event)"` for dirty flags,
///    validation or autosave, exactly as the framework documents.
///
/// `el` is the dispatch origin (`$el` in the template); pass an undefined
/// JsValue where a handler has none and the store write still happens.
fn write_value(scope: &JsValue, el: &JsValue, value: &str) {
    let name = js::get_str(scope, "name");

    let model = js::get(scope, "model");
    if js::is_missing(&model) {
        js::set_str(scope, "localValue", value); // usable without a model
    } else if !name.is_empty() {
        js::set(&model, &name, &JsValue::from_str(value));
    }

    if !name.is_empty() && !js::is_missing(el) && !el.is_null() {
        let detail = js::new_object();
        js::set_str(&detail, "name", &name);
        js::set_str(&detail, "value", value);
        hooks::dispatch(el, FIELD_EVENT, &detail);
    }
}

/// Read the current value: from the host's model when present, else local state.
fn read_value(scope: &JsValue) -> String {
    let model = js::get(scope, "model");
    if js::is_missing(&model) {
        return js::get_str(scope, "localValue");
    }
    let name = js::get_str(scope, "name");
    if name.is_empty() {
        return js::get_str(scope, "localValue");
    }
    let v = js::get(&model, &name);
    if js::is_missing(&v) {
        String::new()
    } else if let Some(s) = v.as_string() {
        s
    } else {
        js::describe(&v)
    }
}

/// Ask the host to submit (Enter in the bar, ⌘Enter in an area).
///
/// Prop keys are the *literal attribute name* — the HTML parser lowercases them,
/// so a camelCase prop set from markup would never be found. Hosts pass
/// `:on-submit="submit"`; both spellings are accepted for programmatic use.
fn emit_submit(scope: &JsValue) {
    for key in ["on-submit", "onsubmit", "onSubmit"] {
        if let Ok(f) = js::get(scope, key).dyn_into::<js_sys::Function>() {
            let _ = f.call0(&JsValue::UNDEFINED);
            return;
        }
    }
}

/// Same reason: read a boolean prop under its lowercased name too.
fn flag(scope: &JsValue, name: &str) -> bool {
    if js::get_bool(scope, name) {
        return true;
    }
    js::get_bool(scope, &name.to_ascii_lowercase())
}

fn value_of(ev: &JsValue) -> String {
    js::get_str(&js::get(ev, "target"), "value")
}

/// `commit($el, $event)` — the ubiquitous "the user typed" handler.
fn commit() -> impl FnMut(JsValue, Array) -> JsValue {
    |this, args| {
        write_value(&this, &js::arg(&args, 0), &value_of(&js::arg(&args, 1)));
        JsValue::UNDEFINED
    }
}

fn own_strings(this: &JsValue, prop: &str) -> Vec<String> {
    js::get(this, prop)
        .dyn_into::<Array>()
        .map(|a| (0..a.length()).map(|i| js::describe(&a.get(i))).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// shared shape
// ---------------------------------------------------------------------------

fn common(d: &mut Def) {
    d.prop_str("name", "")
        .prop_str("model", "")
        .prop_str("label", "")
        .prop_str("hint", "")
        .prop_str("icon", "")
        .prop_str("localValue", "")
        .prop_bool("required", false)
        .prop_bool("bare", false)
        // opt into a wider grid cell — for fields whose content is intrinsically
        // wide (status labels, chip lists) rather than letting them overflow
        .prop_bool("wide", false)
        // the current value, always derived from the host's model
        .getter("value", |this, _args| JsValue::from_str(&read_value(&this)));
}

fn label_line() -> Node {
    m("span").class("field-label").show("label").kids(vec![
        m("i").show("icon").bind("class", "'ph ph-' + icon"),
        m("span").text("label"),
        m("em").class("req").show("required").child(lit("*")),
    ])
}

fn hint_line() -> Node {
    m("span").class("field-hint").show("hint").text("hint")
}

fn field_shell(input: Node) -> Node {
    m("label")
        .class("field")
        .bind("class", "(bare ? 'bare ' : '') + (wide ? 'wide' : '')")
        .kids(vec![label_line(), input, hint_line()])
}

// ---------------------------------------------------------------------------
// uiText
// ---------------------------------------------------------------------------

fn ui_text() {
    let mut d = Def::new().named(TEXT);
    common(&mut d);
    d.prop_str("placeholder", "")
        .prop_bool("submit-on-enter", false)
        .prop_bool("autofocus", false)
        .tree(field_shell(
            m("input")
                .attr("type", "text")
                .attr("autocomplete", "off")
                .attr("spellcheck", "false")
                .bind("placeholder", "placeholder")
                .bind("value", "value")
                // The HTML autofocus attribute only applies during initial
                // insertion, which a diff-driven create cannot rely on — focus
                // it explicitly on init instead.
                .init("autofocus && $el.focus()")
                .bind("aria-label", "label || placeholder")
                .on("input", "commit($el, $event)")
                .on("change", "commit($el, $event)")
                .on("keydown.enter", "press($el, $event)"),
        ))
        .method("commit", commit())
        .method("press", |this, args| {
            write_value(&this, &js::arg(&args, 0), &value_of(&js::arg(&args, 1)));
            if flag(&this, "submit-on-enter") {
                emit_submit(&this);
            }
            JsValue::UNDEFINED
        });
    mjs::component(TEXT, &d.finish());
}

// ---------------------------------------------------------------------------
// uiArea — Enter stays a newline; ⌘/Ctrl+Enter submits
// ---------------------------------------------------------------------------

fn ui_area() {
    let mut d = Def::new().named(AREA);
    common(&mut d);
    d.prop_str("placeholder", "")
        .prop("rows", &JsValue::from(3.0))
        .tree(field_shell(
            m("textarea")
                .attr("spellcheck", "false")
                .bind("rows", "rows")
                .bind("placeholder", "placeholder")
                .bind("value", "value")
                .bind("aria-label", "label || placeholder")
                .on("input", "commit($el, $event)")
                .on("keydown.enter.meta", "press($el, $event)")
                .on("keydown.enter.ctrl", "press($el, $event)"),
        ))
        .method("commit", commit())
        .method("press", |this, args| {
            // write first, so a ⌘Enter never submits a stale value
            write_value(&this, &js::arg(&args, 0), &value_of(&js::arg(&args, 1)));
            emit_submit(&this);
            JsValue::UNDEFINED
        });
    mjs::component(AREA, &d.finish());
}

// ---------------------------------------------------------------------------
// uiNumber — digits are the expensive kind of typing
// ---------------------------------------------------------------------------

fn ui_number() {
    let mut d = Def::new().named(NUMBER);
    common(&mut d);
    d.prop("step", &JsValue::from(1.0))
        .prop("min", &JsValue::from(0.0))
        .tree(field_shell(m("div").class("stepper").kids(vec![
                m("button")
                    .attr("type", "button")
                    .class("step")
                    .attr("aria-label", "decrease")
                    .on("click", "bump($el, -1)")
                    .child(lit("−")),
                m("input")
                    .attr("type", "number")
                    .bind("step", "step")
                    .bind("min", "min")
                    .bind("value", "value")
                    .bind("aria-label", "label || 'value'")
                    .on("input", "commit($el, $event)"),
                m("button")
                    .attr("type", "button")
                    .class("step")
                    .attr("aria-label", "increase")
                    .on("click", "bump($el, 1)")
                    .child(lit("+")),
            ])))
        .method("commit", commit())
        .method("bump", |this, args| {
            let el = js::arg(&args, 0);
            let dir = js::arg_num(&args, 1, 1.0);
            let step = js::get_num(&this, "step").max(1.0);
            let min = js::get_num(&this, "min");
            let current = read_value(&this).parse::<f64>().unwrap_or(0.0);
            let next = (current + dir * step).max(min).max(0.0);
            write_value(&this, &el, &format!("{next:.0}"));
            JsValue::UNDEFINED
        });
    mjs::component(NUMBER, &d.finish());
}

// ---------------------------------------------------------------------------
// uiDate — presets beat both the picker and typing YYYY-MM-DD
// ---------------------------------------------------------------------------

fn ui_date() {
    let mut d = Def::new().named(DATE);
    common(&mut d);
    d.prop_bool("presets", true)
        .tree(field_shell(m("div").class("daterow").kids(vec![
            m("input")
                .attr("type", "date")
                .bind("value", "value")
                .bind("aria-label", "label || 'date'")
                .on("input", "commit($el, $event)")
                .on("change", "commit($el, $event)"),
            m("span").class("presets").show("presetsOn()").kids(vec![
                preset("today", "day"),
                preset("+1w", "week"),
                preset("+1mo", "month"),
                preset("clear", "clear"),
            ]),
        ])))
        .method("commit", commit())
        // Presets are on unless the host switches them off. A valueless attribute
        // parses to "" and `presets="false"` arrives as the *string* "false" —
        // truthy in JS — so the spelling is coerced, not tested for truth. This
        // used to be hand-rolled here; it is `js::flag`, the same coercion every
        // other host-set boolean now goes through. (A doc comment cannot sit
        // inside a call chain, hence //.)
        .method("presetsOn", |this, _args| {
            // one coercion, shared with every other host-set boolean flag
            JsValue::from_bool(js::flag(&this, "presets", true))
        })
        .method("pick", |this, args| {
            let text = match js::arg_str(&args, 1).as_str() {
                "clear" => String::new(),
                "week" => iso_in_days(7),
                "month" => iso_in_days(30),
                _ => iso_in_days(0),
            };
            write_value(&this, &js::arg(&args, 0), &text);
            JsValue::UNDEFINED
        });
    mjs::component(DATE, &d.finish());
}

fn preset(caption: &str, kind: &str) -> Node {
    m("button")
        .attr("type", "button")
        .class("preset")
        .on("click", &format!("pick($el, '{kind}')"))
        .child(lit(caption))
}

/// `YYYY-MM-DD`, n days out — the browser's clock, not a second calendar in Rust.
fn iso_in_days(days: i64) -> String {
    let date_ctor = match js::get(&js::global(), "Date").dyn_into::<js_sys::Function>() {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let now = js::call(&date_ctor, "now", &[]).as_f64().unwrap_or(0.0);
    let when = now + (days as f64) * 86_400_000.0;
    let instance = match Reflect::construct(&date_ctor, &Array::of1(&JsValue::from(when))) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    js::call_string(&instance, "toISOString", &[])
        .chars()
        .take(10)
        .collect()
}

// ---------------------------------------------------------------------------
// uiChoice — a segmented control; one click picks a value
// ---------------------------------------------------------------------------

fn ui_choice() {
    let mut d = Def::new().named(CHOICE);
    common(&mut d);
    d.prop_str("options", "")
        .prop_str("labels", "")
        .prop_str("icons", "")
        // `tones` is positional against `options`, like `labels` and `icons`; a
        // tone name is one of the badge tones (ok | busy | warn | err | info)
        // so the same value reads the same colour here and in a `ui-badge`.
        .prop_str("tones", "")
        .getter("items", |this, _args| {
            let list: Vec<String> = js::get_str(&this, "options")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            array_of(&list).into()
        })
        .getter("pairs", |this, _args| {
            let values = js::get(&this, "items");
            let labels: Vec<String> = js::get_str(&this, "labels")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            let icons: Vec<String> = js::get_str(&this, "icons")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            let tones: Vec<String> = js::get_str(&this, "tones")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            let out = Array::new();
            if let Ok(values) = values.dyn_into::<Array>() {
                for i in 0..values.length() {
                    let value = values.get(i);
                    let text = labels
                        .get(i as usize)
                        .cloned()
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| js::describe(&value));
                    let o = js::new_object();
                    js::set(&o, "value", &value);
                    js::set_str(&o, "text", &text);
                    // `icons` is positional against `options`, like `labels`
                    if let Some(icon) = icons.get(i as usize) {
                        js::set_str(&o, "icon", icon);
                    }
                    if let Some(tone) = tones.get(i as usize).filter(|t| !t.is_empty()) {
                        js::set_str(&o, "tone", tone);
                    }
                    out.push(&o.into());
                }
            }
            out.into()
        })
        .tree(field_shell(
            m("div")
                .class("segmented")
                .attr("role", "group")
                .attr("tabindex", "0")
                .attr("title", "click, or step with the arrow keys")
                .bind("aria-label", "label || name")
                .on("keydown.left", "move($el, -1)")
                .on("keydown.up", "move($el, -1)")
                .on("keydown.right", "move($el, 1)")
                .on("keydown.down", "move($el, 1)")
                // deliberately no `wheel` handler: hovering a control should
                // never change its value — the gesture is page scroll, and an
                // accidental spin silently rewrites a field the user meant to
                // read. Keys are opt-in (the group takes focus on click), so
                // they are the safe fast path.
                // one tooltip anchor per option: the button is its light-DOM
                // slot child, so the bubble positions off that button — and when
                // the option shows an icon instead of a word, the tooltip is
                // what carries the text
                .child(
                    m("span")
                        .widget(crate::ui_tooltip::NAME)
                        .for_each("o in pairs")
                        .key("o.value")
                        .bind("label", "tip(o)")
                        .child(
                            m("button")
                                .attr("type", "button")
                                .class("seg")
                                .bind(
                                    "class",
                                    "'seg' + (o.tone ? ' t-' + o.tone : '') \
                                     + (o.value === value ? ' on' : '')",
                                )
                                .bind("aria-pressed", "o.value === value")
                                .bind("aria-label", "o.text")
                                .on("click", "choose($el, o.value)")
                                .kids(vec![
                                    m("i").show("o.icon").bind("class", "'ph ph-' + o.icon"),
                                    m("span").show("!o.icon").text("o.text"),
                                ]),
                        ),
                ),
        ))
        // A tooltip is only useful when it says something the control does not
        // already show: with icons replacing the words, hand the word to the
        // bubble; in text mode there is nothing to add. (A doc comment cannot
        // appear inside a call chain — rustc reads it as an expression.)
        .method("tip", |this, args| {
            let o = js::arg(&args, 0);
            let show = !js::get_str(&this, "icons").trim().is_empty();
            let text = js::describe(&js::get(&o, "text"));
            JsValue::from_str(if show { &text } else { "" })
        })
        .method("choose", |this, args| {
            write_value(&this, &js::arg(&args, 0), &js::describe(&js::arg(&args, 1)));
            JsValue::UNDEFINED
        })
        .method("move", |this, args| {
            let el = js::arg(&args, 0);
            let dir = js::arg_num(&args, 1, 1.0) as i64;
            let items = own_strings(&this, "items");
            if items.is_empty() {
                return JsValue::UNDEFINED;
            }
            let current = read_value(&this);
            let base = items
                .iter()
                .position(|v| v == &current)
                .map(|i| i as i64)
                .unwrap_or(if dir > 0 { -1 } else { 0 });
            let next = (base + dir).clamp(0, items.len() as i64 - 1) as usize;
            write_value(&this, &el, &items[next]);
            JsValue::UNDEFINED
        });
    mjs::component(CHOICE, &d.finish());
}

fn array_of(list: &[String]) -> Array {
    let a = Array::new();
    for s in list {
        a.push(&JsValue::from_str(s));
    }
    a
}

// ---------------------------------------------------------------------------
// uiChips — tokens instead of a comma-separated string
// ---------------------------------------------------------------------------

fn ui_chips() {
    let mut d = Def::new().named(CHIPS);
    common(&mut d);
    d.prop_str("placeholder", "add…")
        .prop_str("prefix", "")
        .str("pending", "")
        .getter("items", |this, _args| {
            let out = Array::new();
            for piece in read_value(&this).split(',') {
                let t = piece.trim().trim_start_matches('#').trim_start_matches('@');
                if !t.is_empty() {
                    out.push(&JsValue::from_str(t));
                }
            }
            out.into()
        })
        .tree(field_shell(
            m("div").class("chips").on("click", "tap($el)").kids(vec![
                m("span")
                    .class("chip")
                    .for_each("(c, i) in items")
                    .key("c")
                    .kids(vec![
                        m("span").class("chip-t").text("display(c)"),
                        m("button")
                            .attr("type", "button")
                            .class("chip-x")
                            .attr("aria-label", "remove")
                            .on("click", "drop($el, i, $event)")
                            .child(lit("×")),
                    ]),
                m("input")
                    .class("chip-in")
                    .attr("type", "text")
                    .attr("autocomplete", "off")
                    .bind("placeholder", "placeholder")
                    .bind("value", "pending")
                    .bind("aria-label", "label || 'add'")
                    .on("input", "type($el, $event)")
                    .on("keydown.enter.stop.prevent", "add($el)")
                    .on("keydown.space.stop.prevent", "add($el)")
                    .on("keydown.delete", "back($el)")
                    // Commit what was typed when the box loses focus. Without this a
                    // token lives only in `pending` until Enter or Space, so typing
                    // `prtr` and clicking **add task** — the most natural thing in the
                    // world — threw it away: a pointer press blurs the input first, but
                    // nothing listened, and the store never saw the value. Silent data
                    // loss on the happy path, and the field looked like it had accepted
                    // the text because the text was still on screen.
                    //
                    // `change` as well as `focusout`: one fires when the value differs
                    // from the one at focus time, the other always. `add` clears
                    // `pending` before it checks, so a double commit is a no-op rather
                    // than a duplicated chip.
                    .on("focusout", "add($el)")
                    .on("change", "add($el)"),
            ]),
        ))
        .method("display", |this, args| {
            let prefix = js::get_str(&this, "prefix");
            JsValue::from_str(&format!("{prefix}{}", js::describe(&js::arg(&args, 0))))
        })
        .method("type", |this, args| {
            js::set_str(&this, "pending", &value_of(&js::arg(&args, 1)));
            JsValue::UNDEFINED
        })
        .method("add", |this, args| {
            let el = js::arg(&args, 0);
            let token = js::get_str(&this, "pending")
                .trim()
                .trim_start_matches('#')
                .trim_start_matches('@')
                .to_string();
            js::set_str(&this, "pending", "");
            if token.is_empty() {
                return JsValue::UNDEFINED;
            }
            let mut list = own_strings(&this, "items");
            if !list.iter().any(|e| e == &token) {
                list.push(token);
            }
            write_value(&this, &el, &list.join(","));
            JsValue::UNDEFINED
        })
        .method("drop", |this, args| {
            let el = js::arg(&args, 0);
            let idx = js::arg_num(&args, 1, -1.0) as usize;
            let mut list = own_strings(&this, "items");
            if idx < list.len() {
                list.remove(idx);
            }
            write_value(&this, &el, &list.join(","));
            JsValue::UNDEFINED
        })
        .method("back", |this, args| {
            // Backspace in an empty box pops the last chip
            if !js::get_str(&this, "pending").is_empty() {
                return JsValue::UNDEFINED;
            }
            let mut list = own_strings(&this, "items");
            list.pop();
            write_value(&this, &js::arg(&args, 0), &list.join(","));
            JsValue::UNDEFINED
        })
        .method("tap", |_this, args| {
            // clicking anywhere in the box focuses the input, so it is always
            // ready for the next token
            let el = js::arg(&args, 0);
            if let Some(input) = js::query_within(&el, "input.chip-in") {
                js::call(&input, "focus", &[]);
            }
            JsValue::UNDEFINED
        });
    mjs::component(CHIPS, &d.finish());
}

// ---------------------------------------------------------------------------
// uiToggle — one big honest target for a boolean
// ---------------------------------------------------------------------------

fn ui_toggle() {
    let mut d = Def::new().named(TOGGLE);
    common(&mut d);
    d.prop_str("on", "true")
        .prop_str("off", "false")
        .getter("isOn", |this, _args| {
            JsValue::from_bool(read_value(&this) == js::get_str(&this, "on"))
        })
        .tree(m("label").class("field toggle").kids(vec![
                m("button")
                    .attr("type", "button")
                    .class("switch")
                    .attr("role", "switch")
                    .bind("class", "isOn ? 'switch on' : 'switch'")
                    .bind("aria-pressed", "isOn")
                    .on("click", "flip($el)")
                    .child(m("span").class("knob")),
                m("span").class("field-label").kids(vec![
                    m("i").show("icon").bind("class", "'ph ph-' + icon"),
                    m("span").text("label"),
                ]),
                hint_line(),
            ]))
        .method("flip", |this, args| {
            let next = if read_value(&this) == js::get_str(&this, "on") {
                js::get_str(&this, "off")
            } else {
                js::get_str(&this, "on")
            };
            write_value(&this, &js::arg(&args, 0), &next);
            JsValue::UNDEFINED
        });
    mjs::component(TOGGLE, &d.finish());
}

// ---------------------------------------------------------------------------
// host-side sugar — so pages.rs reads like a form, not an attribute list
//
// `model_expr` is the store the field writes into, e.g. `"$store.draft"`.
// ---------------------------------------------------------------------------

fn widget(name: &str, kind: &str, model_expr: &str) -> Node {
    m("span")
        .widget(kind)
        .attr("name", name)
        .bind("model", model_expr)
}

pub fn text(name: &str, label: &str, model_expr: &str, placeholder: &str) -> Node {
    widget(name, TEXT, model_expr)
        .attr("label", label)
        .attr("placeholder", placeholder)
}

/// The fast path: no label, autofocused, Enter submits via `submit_expr`.
pub fn bar(name: &str, model_expr: &str, placeholder: &str, icon: &str, submit_expr: &str) -> Node {
    widget(name, TEXT, model_expr)
        .attr("placeholder", placeholder)
        .attr("icon", icon)
        .attr("bare", "")
        .attr("submit-on-enter", "")
        .attr("autofocus", "")
        .bind("on-submit", submit_expr)
}

pub fn area(
    name: &str,
    label: &str,
    model_expr: &str,
    placeholder: &str,
    rows: f64,
    submit_expr: Option<&str>,
) -> Node {
    let mut node = widget(name, AREA, model_expr)
        .attr("label", label)
        .attr("placeholder", placeholder)
        .attr("rows", &format!("{rows:.0}"));
    if let Some(expr) = submit_expr {
        node = node.bind("on-submit", expr);
    }
    node
}

pub fn number(name: &str, label: &str, model_expr: &str) -> Node {
    widget(name, NUMBER, model_expr).attr("label", label)
}

pub fn date(name: &str, label: &str, model_expr: &str) -> Node {
    widget(name, DATE, model_expr).attr("label", label)
}

pub fn choice(name: &str, label: &str, model_expr: &str, options: &[&str]) -> Node {
    widget(name, CHOICE, model_expr)
        .attr("label", label)
        .attr("options", &options.join(","))
}

/// A segmented control whose options are phosphor icons, each explained by the
/// shared [`ui-tooltip`](crate::ui_tooltip) on hover.
pub fn choice_icons(
    name: &str,
    label: &str,
    model_expr: &str,
    options: &[&str],
    icons: &[&str],
) -> Node {
    widget(name, CHOICE, model_expr)
        .attr("label", label)
        .attr("options", &options.join(","))
        .attr("icons", &icons.join(","))
}

/// Give a segmented control per-option colour, using the badge tone names
/// (`ok | busy | warn | err | info`). Positional against the `options` slice
/// already passed to [`choice`]/[`choice_icons`].
///
/// ```ignore
/// form::tinted(
///     form::choice("priority", "priority", "$store.draft", PRIORITIES),
///     PRIORITY_TONES,
/// )
/// ```
pub fn tinted(node: Node, tones: &[&str]) -> Node {
    node.attr("tones", &tones.join(","))
}

pub fn chips(name: &str, label: &str, model_expr: &str, prefix: &str, placeholder: &str) -> Node {
    widget(name, CHIPS, model_expr)
        .attr("label", label)
        .attr("prefix", prefix)
        .attr("placeholder", placeholder)
}

pub fn toggle(name: &str, label: &str, model_expr: &str) -> Node {
    widget(name, TOGGLE, model_expr).attr("label", label)
}

/// Keyboard hints, so the fast path is discoverable instead of folklore.
pub fn keys(hints: &[(&str, &str)]) -> Node {
    m("span").class("kbd-hints").kids(
        hints
            .iter()
            .map(|(key, what)| {
                m("span")
                    .class("kbd")
                    .kids(vec![m("kbd").child(lit(key)), m("span").child(lit(*what))])
            })
            .collect(),
    )
}
