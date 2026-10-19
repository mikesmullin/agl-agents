//! Small, forgiving wrappers over `js_sys`.
//!
//! Everything the Rust side does with m-js goes through these helpers: the app
//! code stays declarative Rust, and a missing property logs a message instead
//! of trapping the whole wasm instance.
//!
//! Closures are exported with a **fixed** shape — `(this, args: Array)` —
//! because wasm-bindgen has no variadic closures. `hooks::bind` in `app.mjs`
//! is what turns a JS method call into that shape:
//!
//! ```js
//! bind: (fn) => function (...args) { return fn(this, args) }
//! ```

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::prelude::*;

pub fn global() -> JsValue {
    js_sys::global().into()
}

pub fn window() -> web_sys::Window {
    thread_local! {
        static W: web_sys::Window = web_sys::window().expect("rustex1 needs a window");
    }
    W.with(|w| w.clone())
}

pub fn document() -> web_sys::Document {
    window().document().expect("no document")
}

// ---------------------------------------------------------------------------
// reflection
// ---------------------------------------------------------------------------

pub fn get(obj: &JsValue, key: &str) -> JsValue {
    Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

pub fn get_str(obj: &JsValue, key: &str) -> String {
    get(obj, key).as_string().unwrap_or_default()
}

pub fn get_num(obj: &JsValue, key: &str) -> f64 {
    get(obj, key).as_f64().unwrap_or(0.0)
}

pub fn get_bool(obj: &JsValue, key: &str) -> bool {
    get(obj, key).is_truthy()
}

pub fn set(obj: &JsValue, key: &str, value: &JsValue) {
    if Reflect::set(obj, &JsValue::from_str(key), value).is_err() {
        error(&format!("rustex1: could not set `{key}`"));
    }
}

pub fn set_str(obj: &JsValue, key: &str, value: &str) {
    set(obj, key, &JsValue::from_str(value));
}

pub fn set_num(obj: &JsValue, key: &str, value: f64) {
    set(obj, key, &JsValue::from(value));
}

pub fn set_bool(obj: &JsValue, key: &str, value: bool) {
    set(obj, key, &JsValue::from_bool(value));
}

/// Read a boolean that a **host attribute** supplied.
///
/// `get_bool` cannot be used for this: markup has no booleans, so
/// `can-export="false"` arrives as the four-character string `"false"`, and
/// `JsValue::is_truthy` on any non-empty string is `true` — the prop reads as on
/// no matter what the author wrote. A present-but-valueless attribute
/// (`<div can-export>`) arrives as `""`, which by HTML convention also means on.
///
/// This is upstream m-js's `coerceFlag` (`helpers.js:359`) ported. `fallback` is
/// what an *absent* attribute means — the prop's declared default.
///
/// Anything genuinely non-string (a real `JsBoolean`, or an object from a bound
/// `:can-export="expr"`) keeps JS's own truthiness.
pub fn flag(obj: &JsValue, key: &str, fallback: bool) -> bool {
    let v = get(obj, key);
    if let Some(b) = v.as_bool() {
        return b;
    }
    if let Some(s) = v.as_string() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "" | "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            // any other text is still a present attribute, and a present
            // attribute means the author asked for it
            _ => true,
        };
    }
    if is_missing(&v) {
        return fallback;
    }
    v.is_truthy()
}

/// `flag(obj, key, false)` — the common case of "is this switch on".
pub fn flag_on(obj: &JsValue, key: &str) -> bool {
    flag(obj, key, false)
}

pub fn is_missing(v: &JsValue) -> bool {
    v.is_undefined() || v.is_null()
}

pub fn arg(args: &Array, i: usize) -> JsValue {
    if (i as u32) < args.length() {
        args.get(i as u32)
    } else {
        JsValue::UNDEFINED
    }
}

pub fn arg_str(args: &Array, i: usize) -> String {
    arg(args, i).as_string().unwrap_or_default()
}

pub fn arg_num(args: &Array, i: usize, default: f64) -> f64 {
    arg(args, i).as_f64().unwrap_or(default)
}

pub fn typeof_(v: &JsValue) -> &'static str {
    if v.is_undefined() {
        "undefined"
    } else if v.is_null() {
        "null"
    } else if v.is_string() {
        "string"
    } else if v.as_f64().is_some() {
        "number"
    } else if v.as_bool().is_some() {
        "boolean"
    } else if v.is_function() {
        "function"
    } else if v.is_object() {
        "object"
    } else {
        "?"
    }
}

pub fn describe(v: &JsValue) -> String {
    if let Some(s) = v.as_string() {
        return s;
    }
    if v.is_undefined() {
        return "undefined".into();
    }
    if v.is_null() {
        return "null".into();
    }
    if let Some(n) = v.as_f64() {
        return format!("{n}");
    }
    if let Some(b) = v.as_bool() {
        return b.to_string();
    }
    // String(value): turns Errors into "TypeError: …" instead of "[object Object]".
    let string_fn = get(&global(), "String");
    let args = Array::new();
    args.push(v);
    to_function(&string_fn)
        .and_then(|f| f.apply(&JsValue::UNDEFINED, &args).ok())
        .and_then(|x| x.as_string())
        .unwrap_or_else(|| format!("<{}>", typeof_(v)))
}

fn to_function(v: &JsValue) -> Option<Function> {
    v.clone().dyn_into::<Function>().ok()
}

fn args_array(args: &[JsValue]) -> Array {
    let a = Array::new();
    for x in args {
        a.push(x);
    }
    a
}

/// `obj.method(...args)` — failures are reported to the console, never thrown.
pub fn call(obj: &JsValue, method: &str, args: &[JsValue]) -> JsValue {
    let f = get(obj, method);
    match to_function(&f) {
        Some(func) => func.apply(obj, &args_array(args)).unwrap_or_else(|e| {
            error(&format!("rustex1: {method}() threw: {}", describe(&e)));
            JsValue::UNDEFINED
        }),
        None => {
            error(&format!(
                "rustex1: {method} is not callable (typeof {})",
                typeof_(&f)
            ));
            JsValue::UNDEFINED
        }
    }
}

/// `globalThis.method(...args)`
pub fn call_global(method: &str, args: &[JsValue]) -> JsValue {
    let g = global();
    let f = get(&g, method);
    match to_function(&f) {
        Some(func) => func
            .apply(&JsValue::UNDEFINED, &args_array(args))
            .unwrap_or_else(|e| {
                error(&format!(
                    "rustex1: global {method}() threw: {}",
                    describe(&e)
                ));
                JsValue::UNDEFINED
            }),
        None => {
            error(&format!("rustex1: no global `{method}`"));
            JsValue::UNDEFINED
        }
    }
}

/// `obj.method(...args)` returning the result stringified.
pub fn call_string(obj: &JsValue, method: &str, args: &[JsValue]) -> String {
    describe(&call(obj, method, args))
}

pub fn obj() -> Object {
    Object::new()
}

pub fn array_of(items: &[&str]) -> Array {
    let a = Array::new();
    for i in items {
        a.push(&JsValue::from_str(i));
    }
    a
}

// ---------------------------------------------------------------------------
// console
// ---------------------------------------------------------------------------

pub fn log1(v: &JsValue) {
    web_sys::console::log_1(v);
}

pub fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

pub fn warn(msg: &str) {
    web_sys::console::warn_1(&JsValue::from_str(msg));
}

pub fn error(msg: &str) {
    web_sys::console::error_1(&JsValue::from_str(msg));
}

// ---------------------------------------------------------------------------
// closures → JS functions
//
// All of these intentionally leak: the function lives exactly as long as this
// wasm instance does, and a hot swap drops the whole instance anyway.
// ---------------------------------------------------------------------------

/// A this-bound m-js method. Rust sees `(scope, args)`.
pub fn cb<F>(f: F) -> JsValue
where
    F: FnMut(JsValue, Array) -> JsValue + 'static,
{
    let raw =
        Closure::wrap(Box::new(f) as Box<dyn FnMut(JsValue, Array) -> JsValue>).into_js_value();
    crate::hooks::bind(&raw)
}

/// A this-bound method used for its side effects.
pub fn cb_void<F>(mut f: F) -> JsValue
where
    F: FnMut(JsValue, Array) + 'static,
{
    cb(move |this, args| {
        f(this, args);
        JsValue::UNDEFINED
    })
}

/// A plain JS function of one argument (route factories, component factories).
pub fn fn1<F>(f: F) -> JsValue
where
    F: FnMut(JsValue) -> JsValue + 'static,
{
    Closure::wrap(Box::new(f) as Box<dyn FnMut(JsValue) -> JsValue>).into_js_value()
}

/// A plain JS function of no arguments.
pub fn fn0<F>(f: F) -> JsValue
where
    F: FnMut() -> JsValue + 'static,
{
    Closure::wrap(Box::new(f) as Box<dyn FnMut() -> JsValue>).into_js_value()
}

// ---------------------------------------------------------------------------
// DOM sugar — keeps an app off web_sys for the 90% cases
// ---------------------------------------------------------------------------

/// `document.querySelector(sel)`
pub fn query(selector: &str) -> Option<JsValue> {
    let doc = document();
    let el = call(&doc, "querySelector", &[JsValue::from_str(selector)]);
    if is_missing(&el) {
        None
    } else {
        Some(el)
    }
}

/// `document.querySelectorAll(sel)` as an array-like, or `None` when there is
/// nothing to walk.
///
/// The batch form exists so a component that has *N* instances on the page can
/// update all of them with **one** DOM query per tick instead of *N*. For
/// per-second work that difference is the whole cost of the feature.
pub fn query_all(selector: &str) -> Option<JsValue> {
    let doc = document();
    let list = call(&doc, "querySelectorAll", &[JsValue::from_str(selector)]);
    if is_missing(&list) {
        None
    } else {
        Some(list)
    }
}

/// `el.querySelector(sel)` scoped to a subtree.
pub fn query_within(el: &JsValue, selector: &str) -> Option<JsValue> {
    let found = call(el, "querySelector", &[JsValue::from_str(selector)]);
    if is_missing(&found) {
        None
    } else {
        Some(found)
    }
}

/// `el.setAttribute(name, value)` — the one write a frame loop should make.
pub fn set_attr(el: &JsValue, name: &str, value: &str) {
    call(
        el,
        "setAttribute",
        &[JsValue::from_str(name), JsValue::from_str(value)],
    );
}

/// `el.textContent = value`
pub fn set_text(el: &JsValue, value: &str) {
    set(el, "textContent", &JsValue::from_str(value));
}

/// `document.createElement(tag)`
pub fn create(tag: &str) -> JsValue {
    call(&document(), "createElement", &[JsValue::from_str(tag)])
}

pub fn new_object() -> Object {
    Object::new()
}

pub fn new_array() -> Array {
    Array::new()
}

pub fn push(arr: &Array, value: &JsValue) {
    arr.push(value);
}
