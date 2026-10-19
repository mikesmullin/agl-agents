//! Document-level hotkeys.
//!
//! One listener on `document` per wasm instance, installed in `App::started` and
//! unhooked (from `window`) on the next boot so a hot reload leaves exactly one
//! live listener. Each key maps to a **store method** — the same path a click
//! takes:
//!
//!   j / k         focus up / down
//!   l / ;         previous / next page
//!   space / x     select the focused row
//!   enter         open the focused row
//!   a             select all / select none (current page)
//!   q             toggle read / unread
//!   r             run Rules on the target set
//!   z             queue the focused email's recommendation
//!   y             star
//!   p             PULL
//!   u             PUSH
//!   /             focus the search box
//!   esc           close new-rule dialog -> action bar -> open email -> search -> filter

use std::cell::{Cell, RefCell};

use js_sys::{Array, Function};
use wasm_bindgen::prelude::*;

use rustex1_kit::{hooks, js, lifecycle, mjs};

const STORE: &str = "mail";

/// The hotkey table: `(key, store_method, arg)`.
static KEYS: &[(&str, &str, &str)] = &[
    ("j", "moveFocus", "-1"),
    ("k", "moveFocus", "1"),
    ("l", "pagePrev", ""),
    (";", "pageNext", ""),
    (" ", "toggleFocus", ""),
    ("x", "toggleFocus", ""),
    ("enter", "openFocused", ""),
    ("a", "toggleSelectAll", ""),
    ("q", "toggleRead", ""),
    ("r", "evaluateRules", ""),
    ("z", "applyFocusedRec", ""),
    ("y", "act", "star"),
    ("p", "pull", ""),
    ("u", "push", ""),
];

pub fn install() {
    let global = js::global();
    if let Some(stale) = js::get(&global, "__MAIL1_KEY_HANDLER__").dyn_into::<Function>().ok() {
        let doc = js::document();
        let a = vec![JsValue::from_str("keydown"), stale.into()];
        let _ = js::call(&doc, "removeEventListener", &a);
    }
    let jsfn = Closure::wrap(Box::new(|_this: JsValue, args: JsValue| {
        // `hooks::bind` is `function (...args) { return fn(this, args) }`, so a
        // Rust closure behind it always takes **(receiver, arguments-array)**.
        // Written as `|ev|` this listener received `document` — the receiver —
        // and the real KeyboardEvent sat unused in the args array, so every
        // hotkey looked up `document.key` (undefined) and did nothing.
        let ev = args
            .dyn_ref::<Array>()
            .map(|a| a.get(0))
            .unwrap_or(JsValue::UNDEFINED);
        on_key(&ev);
    }) as Box<dyn FnMut(JsValue, JsValue)>).into_js_value();
    let f = hooks::bind(&jsfn);
    let f = f.clone();
    let doc = js::document();
    let a = vec![JsValue::from_str("keydown"), f.clone()];
    let _ = js::call(&doc, "addEventListener", &a);
    js::set(&global, "__MAIL1_KEY_HANDLER__", &f);
    install_mouse_back();
    install_popstate();
}

thread_local! {
    /// The mouse-back gesture that started while an email was open. Closing on
    /// `mousedown` used to clear `openId` before `mouseup` / `auxclick`, so those
    /// later events were not cancelled — and their default action is
    /// `history.back()`. One physical click then left the message **and** walked
    /// history (the preview's `srcdoc` entry, then the page).
    static SWALLOW_BACK: Cell<bool> = const { Cell::new(false) };
    static BACK_CLOSE_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static BACK_HANDLER: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

/// The mouse back button (button 3) navigates whichever frame is focused.
/// The preview iframe is a frame, so that click walks the email's history
/// instead of leaving the message. Catch the whole gesture — `mousedown`,
/// `mouseup`, and `auxclick` — and let exactly one close happen.
fn install_mouse_back() {
    let jsfn = Closure::wrap(Box::new(|_this: JsValue, args: JsValue| {
        let ev = args
            .dyn_ref::<Array>()
            .map(|a| a.get(0))
            .unwrap_or(JsValue::UNDEFINED);
        on_mouse_back(&ev);
    }) as Box<dyn FnMut(JsValue, JsValue)>)
    .into_js_value();
    let handler = hooks::bind(&jsfn);
    let win = js::global();
    if let Some(stale) = js::get(&win, "__MAIL1_BACK_HANDLER__").dyn_into::<Function>().ok() {
        for name in ["mousedown", "mouseup", "auxclick"] {
            let _ = js::call(
                &win,
                "removeEventListener",
                &[JsValue::from_str(name), stale.clone().into(), JsValue::TRUE],
            );
        }
    }
    js::set(&win, "__MAIL1_BACK_HANDLER__", &handler);
    BACK_HANDLER.with(|h| *h.borrow_mut() = Some(handler.clone()));
    let capture = JsValue::TRUE;
    for name in ["mousedown", "mouseup", "auxclick"] {
        let _ = js::call(
            &win,
            "addEventListener",
            &[JsValue::from_str(name), handler.clone(), capture.clone()],
        );
    }
    // `load` does not bubble, but the capture phase on document still sees the
    // preview iframe finish loading. Arm the same trap in that document.
    let handler_for_load = handler.clone();
    let loadfn = Closure::wrap(Box::new(move |_this: JsValue, args: JsValue| {
        let ev = args
            .dyn_ref::<Array>()
            .map(|a| a.get(0))
            .unwrap_or(JsValue::UNDEFINED);
        arm_iframe_back(&ev, &handler_for_load);
    }) as Box<dyn FnMut(JsValue, JsValue)>)
    .into_js_value();
    let load_handler = hooks::bind(&loadfn);
    let doc = js::document();
    let _ = js::call(
        &doc,
        "addEventListener",
        &[
            JsValue::from_str("load"),
            load_handler,
            JsValue::TRUE,
        ],
    );
    if let Some(iframe) = js::query("iframe.raw-html") {
        arm_iframe_el(&iframe, &handler);
    }
}

/// Browser Back should leave the open email, not the app. `openMail` pushes one
/// history entry; this pops it back to a cleared detail view.
fn install_popstate() {
    let win = js::global();
    if let Some(stale) = js::get(&win, "__MAIL1_POPSTATE__").dyn_into::<Function>().ok() {
        let _ = js::call(
            &win,
            "removeEventListener",
            &[JsValue::from_str("popstate"), stale.into()],
        );
    }
    let jsfn = Closure::wrap(Box::new(|_this: JsValue, _args: JsValue| {
        if !lifecycle::am_current() {
            return;
        }
        // The entry we pushed is gone. Clear the detail view, but do not call
        // `history.back()` again — that was the second step.
        crate::store::dismiss_open();
    }) as Box<dyn FnMut(JsValue, JsValue)>)
    .into_js_value();
    let handler = hooks::bind(&jsfn);
    js::set(&win, "__MAIL1_POPSTATE__", &handler);
    let _ = js::call(
        &win,
        "addEventListener",
        &[JsValue::from_str("popstate"), handler],
    );
}

fn on_mouse_back(event: &JsValue) {
    if !lifecycle::am_current() {
        return;
    }
    let button = js::get_num(event, "button") as i32;
    if button != 3 {
        return;
    }
    let mail = mjs::store_ref(STORE);
    let open = !js::is_missing(&mail) && !js::get_str(&mail, "openId").is_empty();
    let swallow = SWALLOW_BACK.with(|c| c.get());
    if !open && !swallow {
        return;
    }
    // Cancel every event in the gesture. The browser navigates on `auxclick`
    // (and some builds on `mouseup`), which is after `mousedown`.
    prevent(event);
    SWALLOW_BACK.with(|c| c.set(true));
    let typ = js::get_str(event, "type");
    if typ != "mouseup" && typ != "auxclick" {
        return;
    }
    if BACK_CLOSE_SCHEDULED.with(|c| c.get()) {
        return;
    }
    BACK_CLOSE_SCHEDULED.with(|c| c.set(true));
    // After the gesture's default action has had its chance. If the browser
    // already went back, `popstate` cleared the email and this is a no-op.
    // If `preventDefault` held, this is the one close.
    lifecycle::after(0.0, || {
        BACK_CLOSE_SCHEDULED.with(|c| c.set(false));
        SWALLOW_BACK.with(|c| c.set(false));
        if !lifecycle::am_current() {
            return;
        }
        let mail = mjs::store_ref(STORE);
        if js::is_missing(&mail) || js::get_str(&mail, "openId").is_empty() {
            return;
        }
        call_store("closeMail", "");
    });
}

/// Write the HTML preview without assigning `srcdoc`. Assigning `srcdoc` is a
/// navigation, and each one is a joint-session-history entry — so Back had to
/// walk the preview and then the page.
pub fn schedule_paint() {
    lifecycle::after(0.0, paint_preview);
}

fn paint_preview() {
    if !lifecycle::am_current() {
        return;
    }
    let Some(iframe) = js::query("iframe.raw-html") else {
        return;
    };
    let mail = mjs::store_ref(STORE);
    if js::is_missing(&mail) {
        return;
    }
    let open = js::get(&mail, "open");
    let html = js::get_str(&open, "bodyHtml");
    let show = js::get_str(&mail, "openMode") == "html"
        && js::get_str(&open, "bodyKind") == "html"
        && !html.is_empty();
    if !show {
        return;
    }
    let sig = format!("{}:{}", js::get_str(&open, "id"), html.len());
    if js::get_str(&iframe, "data-painted") == sig {
        return;
    }
    js::set_str(&iframe, "data-painted", &sig);
    let win = js::get(&iframe, "contentWindow");
    let doc = js::get(&win, "document");
    if js::is_missing(&doc) {
        return;
    }
    let wrapped = format!("<html><body style=\"margin:0\">{html}</body></html>");
    let _ = js::call(&doc, "open", &[]);
    let _ = js::call(&doc, "write", &[JsValue::from_str(&wrapped)]);
    let _ = js::call(&doc, "close", &[]);
    // `document.open` replaced the iframe document, so the trap has to be
    // installed again. `replaceState` keeps that write from becoming a Back step.
    let hist = js::get(&win, "history");
    let _ = js::call(
        &hist,
        "replaceState",
        &[JsValue::NULL, JsValue::from_str(""), JsValue::from_str("about:blank")],
    );
    if let Some(handler) = BACK_HANDLER.with(|h| h.borrow().clone()) {
        arm_iframe_el(&iframe, &handler);
    }
}

fn arm_iframe_back(event: &JsValue, handler: &JsValue) {
    let target = js::get(event, "target");
    let class_name = js::get_str(&target, "className");
    if class_name.contains("raw-html") {
        arm_iframe_el(&target, handler);
    }
}

fn arm_iframe_el(iframe: &JsValue, handler: &JsValue) {
    let win = js::get(iframe, "contentWindow");
    let doc = js::get(&win, "document");
    if js::is_missing(&doc) || !js::is_missing(&js::get(&doc, "__mail1Back")) {
        return;
    }
    js::set(&doc, "__mail1Back", &JsValue::TRUE);
    let capture = JsValue::TRUE;
    for name in ["mousedown", "mouseup", "auxclick"] {
        let _ = js::call(
            &doc,
            "addEventListener",
            &[JsValue::from_str(name), handler.clone(), capture.clone()],
        );
    }
}

fn on_key(event: &JsValue) -> JsValue {
    if !lifecycle::am_current() {
        return JsValue::UNDEFINED;
    }
    let t = js::get(event, "target");
    let tag = if js::is_missing(&t) {
        String::new()
    } else {
        js::get(&t, "tagName").as_string().unwrap_or_default().to_string().to_ascii_uppercase()
    };

    let key = js::get(event, "key").as_string().unwrap_or_default().to_lowercase();

    // Escape closes the New Rule dialog even when the cursor is in its
    // textarea — that field would otherwise swallow the key.
    if key == "escape" && js::get_bool(&mjs::store_ref(STORE), "ruleOpen") {
        call_store("closeNewRule", "");
        prevent(event);
        return JsValue::UNDEFINED;
    }

    // Ignore keys typed inside a form control.
    if matches!(tag.as_str(), "INPUT" | "TEXTAREA" | "SELECT") {
        return JsValue::UNDEFINED;
    }


    if let Some((_want, method, arg)) = KEYS.iter().find(|(k, _, _)| *k == key) {
        call_store(method, arg);
        prevent(event);
        return JsValue::UNDEFINED;
    }
    match key.as_str() {
        "/" => {
            if let Some(el) = js::query("[data-search]") {
                let _ = js::call(&el, "focus", &[]);
                let _ = js::call(&el, "select", &[]);
            }
            prevent(event);
        }
        "escape" => {
            close_chain(&mjs::store_ref(STORE));
            prevent(event);
        }
        _ => {}
    }
    JsValue::UNDEFINED
}

fn call_store(method: &str, arg: &str) {
    let mail = mjs::store_ref(STORE);
    let f = js::get(&mail, method);
    let args = Array::new();
    if !arg.is_empty() {
        args.push(&JsValue::from_str(arg));
    }
    match f.clone().dyn_into::<Function>() {
        Ok(func) => {
            if let Err(e) = func.apply(&mail, &args) {
                rustex1_kit::session::toast(
                    &format!("store.{}() threw: {}", method, js::describe(&e)),
                    "err",
                );
            }
        }
        Err(e) => {
            rustex1_kit::session::toast(
                &format!("store method {} not found: {}", method, js::describe(&e)),
                "err",
            );
        }
    }
}

fn close_chain(mail: &JsValue) {
    let ui = mjs::store_ref("ui");
    if js::is_missing(&ui) || !js::get_bool(&ui, "open") {
        if js::is_missing(mail) {
            return;
        }
        if !js::get_str(mail, "openId").is_empty() {
            call_store("closeMail", "");
            return;
        }
        if !js::get_str(mail, "q").is_empty() {
            // A search is a server query now, so clearing the text is not enough
            // — `clearSearch` re-polls the unfiltered list.
            call_store("clearSearch", "");
            return;
        }
        call_store("clearSelection", "");
        call_store("setFilter", "inbox");
        js::set_str(mail, "focusId", "");
    }
}

fn prevent(event: &JsValue) {
    let _ = js::call(event, "preventDefault", &[]);
    let _ = js::call(event, "stopPropagation", &[]);
}
