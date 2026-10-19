//! Bindings for the slice of m-js this app drives, plus a builder for
//! component/store definitions.
//!
//! The `M` namespace object is handed to `boot()` by `app.mjs` — m-js is loaded
//! once from the CDN and the *same* instance survives every hot swap, which is
//! why stores and the router keep working across rebuilds.

use std::cell::RefCell;

use js_sys::{Array, Object};
use wasm_bindgen::prelude::*;

use crate::js;

thread_local! {
    static NS: RefCell<JsValue> = const { RefCell::new(JsValue::UNDEFINED) };
    static ROUTER: RefCell<JsValue> = const { RefCell::new(JsValue::UNDEFINED) };
    /// Last template registered under each name, for `dumpTemplate`.
    static TEMPLATES: RefCell<std::collections::HashMap<String, String>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The template m-js was handed for a component, if anything registered it.
pub fn template_of(name: &str) -> String {
    TEMPLATES.with(|t| {
        t.borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("(no component named {name} registered)"))
    })
}

fn remember(name: &str, def: &JsValue) {
    let tpl = js::get_str(def, "template");
    if !tpl.is_empty() {
        TEMPLATES.with(|t| {
            t.borrow_mut().insert(name.to_string(), tpl);
        });
    }
}

pub fn install(m: JsValue, router: JsValue) {
    NS.with(|n| *n.borrow_mut() = m);
    ROUTER.with(|r| *r.borrow_mut() = router);
}

pub fn m() -> JsValue {
    NS.with(|n| n.borrow().clone())
}

pub fn router() -> JsValue {
    ROUTER.with(|r| r.borrow().clone())
}

// ---------------------------------------------------------------------------
// M.*
// ---------------------------------------------------------------------------

/// `M.component(name, def)` — def is an object (shared prototype) or a factory.
pub fn component(name: &str, def: &JsValue) {
    remember(name, def);
    NAMES.with(|n| n.borrow_mut().insert(name.to_string()));
    js::call(&m(), "component", &[JsValue::from_str(name), def.clone()]);
}

thread_local! {
    /// Every widget name this instance registered — the linter checks
    /// `x-component="…"` against it, because an unknown name is only a
    /// `console.warn` at render time.
    static NAMES: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

pub fn has_component(name: &str) -> bool {
    NAMES.with(|n| n.borrow().contains(name))
}

/// Factory form: `make` runs once per instance, so the widget owns its state.
pub fn component_factory<F>(name: &str, make: F)
where
    F: FnMut(JsValue) -> JsValue + 'static,
{
    // Factory-registered widgets must join the name set the linter checks
    // `x-component="<name>"` against — otherwise a legitimately registered
    // widget reads as "unknown", which both cries wolf and masks the genuine
    // typos the rule exists to catch. (Found exactly this way: ui-tooltip is a
    // factory def, and the boot overlay lit up while the tooltip worked.)
    NAMES.with(|n| n.borrow_mut().insert(name.to_string()));
    js::call(&m(), "component", &[JsValue::from_str(name), js::fn1(make)]);
}

/// `M.store(name, def)` — merges new methods onto the HMR-persistent bucket.
pub fn store(name: &str, def: &JsValue) -> JsValue {
    STORES.with(|s| s.borrow_mut().insert(name.to_string()));
    js::call(&m(), "store", &[JsValue::from_str(name), def.clone()])
}

thread_local! {
    static STORES: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

/// Did this instance register a store under `name`? The linter resolves every
/// `$store.<name>` in every template against this.
pub fn has_store(name: &str) -> bool {
    STORES.with(|s| s.borrow().contains(name))
}

/// `M.store(name)` — the reactive proxy; writing a property schedules a redraw.
pub fn store_ref(name: &str) -> JsValue {
    js::call(&m(), "store", &[JsValue::from_str(name)])
}

pub fn mount(selector: &str) {
    js::call(&m(), "mount", &[JsValue::from_str(selector)]);
}

/// Tear the rendered tree down without touching store data. Called on a hot
/// swap so every widget is re-instantiated from the *new* definitions — m-js
/// keeps existing component instances alive (and with them their old template
/// string) unless they are unmounted first.
pub fn unmount() {
    js::call(&m(), "unmount", &[]);
}

pub fn redraw() {
    js::call(&m(), "redraw", &[]);
}

// ---------------------------------------------------------------------------
// Router.*
// ---------------------------------------------------------------------------

pub fn router_set_mode(mode: &str) {
    js::call(&router(), "setMode", &[JsValue::from_str(mode)]);
}

/// `Router.register(uri, title, fn(params) => component)`
pub fn router_register<F>(uri: &str, title: &str, make: F)
where
    F: FnMut(JsValue) -> JsValue + 'static,
{
    js::call(
        &router(),
        "register",
        &[
            JsValue::from_str(uri),
            JsValue::from_str(title),
            js::fn1(make),
        ],
    );
}

pub fn router_href(uri: &str) -> String {
    js::describe(&js::call(&router(), "href", &[JsValue::from_str(uri)]))
}

pub fn router_link(event: &JsValue) {
    js::call(&router(), "link", &[event.clone()]);
}

pub fn router_set(uri: &str) {
    js::call(&router(), "set", &[JsValue::from_str(uri)]);
}

pub fn router_uri() -> String {
    js::describe(&js::get(&router(), "uri"))
}

// ---------------------------------------------------------------------------
// definition builder
// ---------------------------------------------------------------------------

/// Builds the plain JS object m-js expects for `M.component` / `M.store`.
pub struct Def {
    o: Object,
    props: Vec<String>,
    label: String,
}

impl Def {
    pub fn new() -> Def {
        Def {
            o: Object::new(),
            props: Vec::new(),
            label: "anonymous".to_string(),
        }
    }

    /// Name this definition so linter findings point at something useful.
    pub fn named(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }

    pub fn template(&mut self, html: &str) -> &mut Def {
        js::set_str(&self.o, "template", html);
        self
    }

    /// The hyperscript form: hand over an `m()` tree and let the serializer
    /// produce the template string. Nothing here can mis-close a tag, because
    /// there are no closing tags to write.
    pub fn tree(&mut self, node: crate::h::Node) -> &mut Def {
        crate::h::record(&self.label, &node);
        js::set_str(&self.o, "template", &node.into_html());
        self
    }

    /// A template made of several root nodes.
    pub fn trees(&mut self, nodes: Vec<crate::h::Node>) -> &mut Def {
        for node in &nodes {
            crate::h::record(&self.label, node);
        }
        js::set_str(&self.o, "template", &crate::h::frag(nodes));
        self
    }

    /// A declared prop (copied from the host element) with a default, so the
    /// template can always resolve the binding.
    pub fn prop(&mut self, name: &str, default: &JsValue) -> &mut Def {
        self.props.push(name.to_string());
        js::set(&self.o, name, default);
        self
    }

    pub fn prop_str(&mut self, name: &str, default: &str) -> &mut Def {
        self.prop(name, &JsValue::from_str(default))
    }

    pub fn prop_num(&mut self, name: &str, default: f64) -> &mut Def {
        self.prop(name, &JsValue::from(default))
    }

    pub fn prop_bool(&mut self, name: &str, default: bool) -> &mut Def {
        self.prop(name, &JsValue::from_bool(default))
    }

    /// Plain (non-prop) state.
    pub fn val(&mut self, name: &str, value: &JsValue) -> &mut Def {
        js::set(&self.o, name, value);
        self
    }

    pub fn str(&mut self, name: &str, value: &str) -> &mut Def {
        self.val(name, &JsValue::from_str(value))
    }

    pub fn num(&mut self, name: &str, value: f64) -> &mut Def {
        self.val(name, &JsValue::from(value))
    }

    /// A method. Rust sees `(scope, args)` — `this` arrives as the first
    /// parameter thanks to `hooks::bind`.
    pub fn method<F>(&mut self, name: &str, f: F) -> &mut Def
    where
        F: FnMut(JsValue, js_sys::Array) -> JsValue + 'static,
    {
        js::set(&self.o, name, &js::cb(f));
        self
    }

    /// A computed property: `Object.defineProperty(this, name, { get })`.
    /// m-js evaluates template expressions against the scope, so a getter is how
    /// a widget derives a view (`items`, `pairs`, `isOn`) from its props without
    /// the host having to supply it.
    /// The closure is called like a method — `(receiver, args)` — because a JS
    /// getter receives no arguments: the scope arrives as `this`, which only
    /// `hooks::bind` can capture.
    pub fn getter<F>(&mut self, name: &str, get: F) -> &mut Def
    where
        F: FnMut(JsValue, js_sys::Array) -> JsValue + 'static,
    {
        let descriptor = js::obj();
        js::set(&descriptor, "get", &js::cb(get));
        js::set_bool(&descriptor, "configurable", true);
        js::set_bool(&descriptor, "enumerable", true);
        let _ = js_sys::Object::define_property(&self.o, &JsValue::from_str(name), &descriptor);
        self
    }

    /// A method used for its side effects.
    pub fn action<F>(&mut self, name: &str, f: F) -> &mut Def
    where
        F: FnMut(JsValue, js_sys::Array) + 'static,
    {
        js::set(&self.o, name, &js::cb_void(f));
        self
    }

    pub fn bool(&mut self, name: &str, value: bool) -> &mut Def {
        self.val(name, &JsValue::from_bool(value))
    }

    pub fn js(&self) -> JsValue {
        self.o.clone().into()
    }

    pub fn finish(&mut self) -> JsValue {
        if !self.props.is_empty() {
            let a = Array::new();
            for p in &self.props {
                a.push(&JsValue::from_str(p));
            }
            js::set(&self.o, "props", &a.into());
        }
        self.o.clone().into()
    }
}

/// Convenience: build a component from a template + declared props in one call.
pub fn simple_component(name: &str, build: impl FnOnce(&mut Def)) {
    let mut def = Def::new();
    build(&mut def);
    component(name, &def.finish());
}
