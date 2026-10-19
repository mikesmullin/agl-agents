//! Routes. The same shell renders for both `/` and `/mail/:id`; the detail
//! region is toggled by `$store.mail.openId`. The toolbar sits above both,
//! just like the reference email-trainer-2 (sidebar and training pages omitted).

use rustex1_kit::h::{lit, m, Node};
use rustex1_kit::{js, mjs, mjs::Def};
use wasm_bindgen::prelude::*;
use js_sys::{Array, Date, Function};

/// Read a string field off an object value.
fn fstr(obj: &JsValue, key: &str) -> String {
    js::get_str(obj, key)
}

/// One queued mutation, in the words the user used to ask for it:
/// `instruction` is the action pressed (the server defaults it to the op when
/// the UI sent none), while `op` is what Gmail will be asked to do — "modify"
/// tells a reader nothing.
fn pending_op(p: &JsValue) -> String {
    let mut op = fstr(p, "instruction");
    if op.is_empty() {
        op = fstr(p, "op");
    }
    op
}

/// `names` is an array of **strings**, not of `{name}` objects.
fn pending_names(p: &JsValue) -> String {
    let names = js::get(p, "names");
    let mut nm = String::new();
    if let Some(arr) = names.dyn_ref::<js_sys::Array>() {
        for j in 0..arr.length() {
            if j > 0 {
                nm.push_str(", ");
            }
            nm.push_str(&arr.get(j).as_string().unwrap_or_default());
        }
    }
    nm
}

pub fn shell_with_open(params: JsValue) -> JsValue {
    let id = fstr(&params, "id");
    if !id.is_empty() {
        let mail = mjs::store_ref("mail");
        js::set_str(&mail, "openId", &id);
        crate::store::open_message(&id);
    }
    shell()
}

pub fn register() {
    mjs::router_set_mode("path");
    mjs::router_register("/", "mail1 · inbox", |_| shell());
    mjs::router_register("/mail/:id", "mail1 · mail", |params| shell_with_open(params));
}

fn is_selected(mail: &JsValue, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    // `selected` is a JS **array of ids** (see store.rs::set_selected), so this
    // must read it as an array. Reading it as an object — `arr[id]` — finds a
    // property named after the id on an array, which is always undefined, and
    // every row then looks unselected no matter what the store holds.
    let sel = js::get(mail, "selected");
    match sel.dyn_ref::<js_sys::Array>() {
        Some(a) => (0..a.length()).any(|i| a.get(i).as_string().unwrap_or_default() == id),
        None => false,
    }
}

/// The server's own answer to "what does this row look like right now":
/// `unread` / `starred` / `important` are computed from `effLabels`, which is
/// labels + pending.add − pending.remove. (`effLabels` is an array of
/// **strings**, so `effLabels[i].name` is undefined and every such test used to
/// answer "no" — which is why unread rows never looked unread.)
fn row_flag(row: &JsValue, field: &str) -> bool {
    js::get_bool(row, field)
}

/// Call a method on the mail store with one string argument (or none).
fn call_store(method: &str, arg: &str) {
    let mail = mjs::store_ref("mail");
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
                &format!("store method {} is not a function: {}", method, js::describe(&e)),
                "err",
            );
        }
    }
}

fn set_mail(field: &str, val: &str) {
    let mail = mjs::store_ref("mail");
    js::set_str(&mail, field, val);
}

/// Open only when the click landed on the sender, the subject, or the summary.
/// The subject and snippet live inside `.summary`; the grid gap and the empty
/// part of the recommendation column land on the row itself.
fn click_opens(ev: &JsValue) -> bool {
    let mut node = js::get(ev, "target");
    if js::get_num(&node, "nodeType") == 3.0 {
        node = js::get(&node, "parentElement");
    }
    if js::is_missing(&node) {
        return false;
    }
    let found = js::call(
        &node,
        "closest",
        &[JsValue::from_str(".sender, .summary")],
    );
    !js::is_missing(&found)
}

pub fn shell() -> JsValue {
    let mut d = Def::new().named("route /");
    d.tree(m("div")
        .class("mail-shell")
        .kids(vec![
            topbar(),
            search_bar(),
            tool_bar(),
            action_bar(),
            new_rule_dialog(),
            controls_row(),
            list_region(),
            detail_region(),
            hint_strip(),
            Node::host("toastStack", &[]),
        ]));

    // ── page methods ─────────────────────────────────────────────────────
    // Sender / subject / summary open the email (shift-click ranges from the
    // anchor instead). The checkbox toggles selection. Blank row padding —
    // especially beside the recommendation button — does neither.
    d.method("rowClick", |_this, args| {
        let r = args.get(0);
        let id = fstr(&r, "id");
        let ev = args.get(1);
        let shift = js::get(&ev, "shiftKey").as_bool().unwrap_or(false);
        if shift {
            call_store("selectRange", &id);
            set_mail("focusId", &id);
            return JsValue::UNDEFINED;
        }
        if click_opens(&ev) {
            call_store("openMail", &id);
            set_mail("focusId", &id);
        }
        JsValue::UNDEFINED
    })
    .method("sel", |_this, args| {
        let r = args.get(0);
        let id = fstr(&r, "id");
        let ev = args.get(1);
        let shift = js::get(&ev, "shiftKey").as_bool().unwrap_or(false);
        if shift {
            let _ = js::call(&ev, "preventDefault", &[]);
            call_store("selectRange", &id);
        } else {
            call_store("selectOnly", &id);
        }
        set_mail("focusId", &id);
        JsValue::UNDEFINED
    })
    .method("rowClass", |_this, args| {
        let r = args.get(0);
        let mail = mjs::store_ref("mail");
        let id = fstr(&r, "id");
        let mut t = String::new();
        if is_selected(&mail, &id) {
            t.push_str(" is-sel");
        }
        if fstr(&mail, "focusId") == id {
            t.push_str(" is-focus");
        }
        if row_flag(&r, "unread") {
            t.push_str(" is-unread");
        }
        if row_flag(&r, "starred") {
            t.push_str(" is-starred");
        }
        if row_flag(&r, "important") {
            t.push_str(" is-important");
        }
        if js::get_num(&r, "pendingCount") as i64 > 0 {
            t.push_str(" is-pending");
        }
        if !js::is_missing(&js::get(&r, "phrase")) {
            t.push_str(" is-phrased");
        }
        JsValue::from_str(&t)
    })
    .method("pendingChip", |_this, args| {
        let r = args.get(0);
        let pending = js::get(&r, "pending");
        let mut parts = Vec::new();
        if let Some(a) = pending.dyn_ref::<js_sys::Array>() {
            for i in 0..a.length() {
                let p = a.get(i);
                let op = pending_op(&p);
                let nm = pending_names(&p);
                if op == "move" && !nm.is_empty() {
                    parts.push(format!("move→{}", nm));
                } else {
                    parts.push(op);
                }
            }
        }
        if parts.is_empty() {
            JsValue::UNDEFINED
        } else {
            JsValue::from_str(&format!("⏳ {}", parts.join(", ")))
        }
    })
    .method("checkClass", |_this, args| {
        let r = args.get(0);
        let mail = mjs::store_ref("mail");
        let id = fstr(&r, "id");
        if is_selected(&mail, &id) {
            JsValue::from_str(" on")
        } else {
            JsValue::from_str("")
        }
    })
    // The select-all button shows a filled box once *every* row is selected.
    .method("selAllClass", |_this, _args| {
        let mail = mjs::store_ref("mail");
        if js::get_bool(&mail, "allSelected") {
            JsValue::from_str(" on")
        } else {
            JsValue::from_str("")
        }
    })
    // Which filter chip is lit. `filter` is the store's, not the chip's, so the
    // esc key and a chip can never disagree.
    // The New Rule narration, read from the textarea itself. The store's
    // `rulePrompt` is the model binding, but a poll redraw can rebuild the
    // textarea and drop keystrokes that landed between frames — so the submit
    // reads the live element, which always holds what the user actually typed.
    .method("ruleNarration", |_this, _args| {
        let el = js::get(&js::global(), "document");
        let q = js::get(&el, "querySelector");
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str("input.rule-prompt"));
        let node = match q.clone().dyn_into::<js_sys::Function>() {
            Ok(f) => f.apply(&el, &args).unwrap_or(JsValue::UNDEFINED),
            Err(_) => JsValue::UNDEFINED,
        };
        let live = js::get_str(&node, "value");
        if !live.trim().is_empty() {
            return JsValue::from_str(&live);
        }
        JsValue::from_str(&js::get_str(&mjs::store_ref("mail"), "rulePrompt"))
    })
    .method("filterClass", |_this, args| {
        let want = args.get(0).as_string().unwrap_or_default();
        let mail = mjs::store_ref("mail");
        if js::get_str(&mail, "filter") == want.as_str() {
            JsValue::from_str(" on")
        } else {
            JsValue::from_str("")
        }
    })
    .method("pendingOp", |_this, args| JsValue::from_str(&pending_op(&args.get(0))))
    .method("pendingNames", |_this, args| JsValue::from_str(&pending_names(&args.get(0))))
    .method("starClass", |_this, args| {
        let r = args.get(0);
        if row_flag(&r, "starred") {
            JsValue::from_str(" on")
        } else {
            JsValue::from_str("")
        }
    })
    .method("impClass", |_this, args| {
        let r = args.get(0);
        if row_flag(&r, "important") {
            JsValue::from_str(" on")
        } else {
            JsValue::from_str("")
        }
    })
    // The row's important glyph: a dim warning **circle** until the email is
    // important, then the filled triangle. A bare triangle on every row reads as
    // "this mail is flagged", which is the opposite of what an off-state icon
    // is for.
    .method("impIcon", |_this, args| {
        let r = args.get(0);
        JsValue::from_str(if row_flag(&r, "important") {
            "warning"
        } else {
            "warning-circle"
        })
    })
    // The label each row carries, in the words a person would recognise.
    .method("rowLabels", |_this, args| {
        let out = Array::new();
        for name in row_label_names(&args.get(0)) {
            out.push(&JsValue::from_str(&name));
        }
        out.into()
    })
    .method("labelChipClass", |_this, args| {
        let name = args.get(0).as_string().unwrap_or_default();
        JsValue::from_str(&label_chip_class(&name))
    })
    // A user label's colour, as an inline style. Gmail's REST API does **not**
    // return one — `labels.list` and `labels.get` for this account carry no
    // `textColorLabel` / `backgroundColorLabel` at all — so the mirror cannot
    // copy the colour the user picked. What it can do is derive a stable hue
    // from the label's name: the same label is the same colour in every row, and
    // `Job/HSA` no longer looks like `Job/Blizzard`. Categories and the gone
    // folders keep the colours styles.css already gives them.
    .method("labelStyle", |_this, args| {
        let name = args.get(0).as_string().unwrap_or_default();
        let style = label_style(&name);
        if style.is_empty() {
            JsValue::UNDEFINED
        } else {
            JsValue::from_str(&style)
        }
    })
    // ── the rule recommendation on a row ───────────────────────────────
    .method("hasRec", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        let rec = rec_of(&id);
        JsValue::from(!js::is_missing(&rec) && !fstr(&rec, "action").is_empty())
    })
    // The row button's phase. Untouched → Rules. Ran, nothing to do → New Rule.
    // A recommendation waiting → the action. That action queued → Undo.
    .method("phaseRules", |_this, args| JsValue::from(row_phase(&arg0(&args)) == "rules"))
    .method("phaseNew", |_this, args| JsValue::from(row_phase(&arg0(&args)) == "newrule"))
    .method("phaseRec", |_this, args| JsValue::from(row_phase(&arg0(&args)) == "rec"))
    .method("phaseSkip", |_this, args| JsValue::from(row_phase(&arg0(&args)) == "skip"))
    .method("phaseUndo", |_this, args| JsValue::from(row_phase(&arg0(&args)) == "undo"))
    .method("ruleScope", |_this, _args| {
        let mail = mjs::store_ref("mail");
        JsValue::from_str(if js::get_str(&mail, "ruleRow").is_empty() {
            "for the selected emails"
        } else {
            "for this email"
        })
    })
    .method("recLabel", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        JsValue::from_str(&fstr(&rec_of(&id), "label"))
    })
    // The row button's text: the destination or the verb alone. The gradient
    // already says *which kind* of action this is (violet = move, red =
    // destructive, amber = skip), the wand says it is a recommendation, and
    // the full `move→Expenses` stays in the title — so the button never needs
    // the prefix that made it overflow its column.
    .method("recShort", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        let rec = rec_of(&id);
        let folder = fstr(&rec, "folder");
        let short = if folder.is_empty() { fstr(&rec, "action") } else { folder };
        JsValue::from_str(&short)
    })
    .method("recClass", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        let rec = rec_of(&id);
        let action = fstr(&rec, "action");
        let low = if js::get_bool(&rec, "ok") { "" } else { " rec-bad" };
        JsValue::from_str(&format!(" rec-{action}{low}"))
    })
    // The title is where the honesty lives: the confidence, which rules matched,
    // and how long the model took. A bare "archive" button with no provenance
    // reads like a decision the app made.
    .method("recTitle", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        let rec = rec_of(&id);
        if js::is_missing(&rec) {
            return JsValue::UNDEFINED;
        }
        let mut parts = vec![fstr(&rec, "label")];
        if !js::is_missing(&rec) {
            parts.push(format!("confidence {:.2}", js::get_num(&rec, "confidence")));
        }
        let rules = js::get(&rec, "rules");
        if let Some(a) = rules.dyn_ref::<js_sys::Array>() {
            let names: Vec<String> = (0..a.length()).map(|i| a.get(i).as_string().unwrap_or_default()).collect();
            if !names.is_empty() {
                parts.push(format!("rules: {}", names.join(", ")));
            }
        }
        let ms = js::get_num(&rec, "ms");
        if ms > 0.0 {
            parts.push(format!("{ms:.0} ms"));
        }
        let err = fstr(&rec, "error");
        if !err.is_empty() {
            parts.push(err);
        }
        JsValue::from_str(&parts.join(" · "))
    })
    // One line per prompt the rules asked: choice votes and noul/score signals
    // alike. The server preformats them so a score of 2.40 and a noul of 0.88
    // read as themselves instead of being squeezed into a choice shape.
    // ── reworded rows ────────────────────────────────────────────────
    .method("isPhrased", |_this, args| {
        JsValue::from(!js::is_missing(&js::get(&args.get(0), "phrase")))
    })
    .method("subjText", |_this, args| {
        let r = args.get(0);
        let p = js::get(&r, "phrase");
        let s = fstr(&p, "subject");
        let orig = fstr(&r, "subject");
        JsValue::from_str(if s.is_empty() { &orig } else { &s })
    })
    .method("sumText", |_this, args| {
        let r = args.get(0);
        let p = js::get(&r, "phrase");
        let s = fstr(&p, "summary");
        let orig = fstr(&r, "snippet");
        JsValue::from_str(if s.is_empty() { &orig } else { &s })
    })
    // Provenance on hover: which phraser wrote it, and what the row used to say.
    .method("subjTitle", |_this, args| {
        let r = args.get(0);
        let p = js::get(&r, "phrase");
        if js::is_missing(&p) {
            return JsValue::UNDEFINED;
        }
        JsValue::from_str(&format!(
            "reworded by {} — was: {}",
            fstr(&p, "rule"),
            fstr(&r, "subject")
        ))
    })
    // Whether any row on screen carries a rewording — the clear button's show.
    .method("anyPhrased", |_this, _args| {
        let mail = mjs::store_ref("mail");
        let rows = js::get(&mail, "rows");
        let any = rows
            .dyn_ref::<js_sys::Array>()
            .map(|a| (0..a.length()).any(|i| !js::is_missing(&js::get(&a.get(i), "phrase"))))
            .unwrap_or(false);
        JsValue::from(any)
    })
    .method("openSubject", |_this, _args| {
        let mail = mjs::store_ref("mail");
        let open = js::get(&mail, "open");
        let p = js::get(&open, "phrase");
        let s = fstr(&p, "subject");
        let orig = fstr(&open, "subject");
        JsValue::from_str(if s.is_empty() { &orig } else { &s })
    })
    .method("recRuleNames", |_this, args| {
        let id = args.get(0).as_string().unwrap_or_default();
        let out = Array::new();
        if let Some(a) = js::get(&rec_of(&id), "answers").dyn_ref::<js_sys::Array>() {
            for i in 0..a.length() {
                let line = a.get(i).as_string().unwrap_or_default();
                if !line.is_empty() {
                    out.push(&JsValue::from_str(&line));
                }
            }
        }
        out.into()
    })
    // "4h", "3d", "4mo" — with the full timestamp in the title. An absolute
    // `2026-10-18 04:36` in every row makes the reader do the subtraction the
    // list exists to do for them.
    .method("timeText", |_this, args| {
        JsValue::from_str(&compact_time(js::get_num(&args.get(0), "dateMs")))
    })
    .method("timeTitle", |_this, args| JsValue::from_str(&fstr(&args.get(0), "dateIso")))
    .method("rawDoc", |_this, _args| {
        let mail = mjs::store_ref("mail");
        let mode = fstr(&mail, "openMode");
        // The body lives on the *open message*, not on the store: `mail.bodyHtml`
        // is undefined, which made this return nothing and left the iframe blank.
        let open = js::get(&mail, "open");
        let html = fstr(&open, "bodyHtml");
        if mode == "html" && !html.is_empty() {
            JsValue::from_str(&format!("<html><body style=\"margin:0\">{}</body></html>", html))
        } else {
            JsValue::UNDEFINED
        }
    })
    .method("modeClass", |_this, args| {
        let mode = args
            .get(0)
            .as_string()
            .unwrap_or_default();
        let mail = mjs::store_ref("mail");
        let cur = fstr(&mail, "openMode");
        if cur == mode {
            JsValue::from_str(" active")
        } else {
            JsValue::from_str("")
        }
    })
    .finish()
}

// ── chrome builders ──────────────────────────────────────────────────────────

/// A message's recommendation, if the rules have produced one.
fn rec_of(id: &str) -> JsValue {
    let mail = mjs::store_ref("mail");
    let recs = js::get(&mail, "recs");
    js::get(&recs, id)
}

fn arg0(args: &js_sys::Array) -> String {
    args.get(0).as_string().unwrap_or_default()
}

/// `rules` until this email has been asked. `newrule` when it was asked and
/// nothing recommended. `rec` while a recommendation is waiting. `undo` after
/// that recommendation was queued.
fn row_phase(id: &str) -> &'static str {
    let mail = mjs::store_ref("mail");
    if js::get_bool(&js::get(&mail, "undoIds"), id) {
        return "undo";
    }
    let rec = rec_of(id);
    if js::is_missing(&rec) {
        return "rules";
    }
    if fstr(&rec, "action").is_empty() {
        "newrule"
    } else if fstr(&rec, "action") == "skip" {
        // skip is not an action. The row shows a label, not a button.
        "skip"
    } else {
        "rec"
    }
}

/// Gmail's own labels are already drawn by the row: INBOX is which chip is lit,
/// UNREAD is the bold sender, STARRED / IMPORTANT are the two icons. What a row
/// is missing is the label **a person** gave it, plus Gmail's category.
const HIDDEN_LABELS: &[&str] = &[
    "INBOX",
    "UNREAD",
    "STARRED",
    "IMPORTANT",
    "SENT",
    "DRAFT",
    "CHAT",
    "CATEGORY_PERSONAL",
    // Gmail's coloured stars/labels are system labels, not folders a person
    // made: a row already shows its star, and a `YELLOW_STAR` chip says nothing
    // a reader can act on.
    "YELLOW_STAR",
    "ORANGE_STAR",
    "RED_STAR",
    "PINK_STAR",
    "PURPLE_STAR",
    "BLUE_STAR",
    "GREEN_STAR",
    "GREEN_CHECK",
    "GREEN_PIN",
    "GREEN_FLAG",
    "GREEN_LABEL",
    "RED_CIRCLE",
    "ORANGE_CIRCLE",
];

fn category_name(label: &str) -> Option<&'static str> {
    match label {
        "CATEGORY_UPDATES" | "updates" => Some("updates"),
        "CATEGORY_PROMOTIONS" | "promotions" => Some("promotions"),
        "CATEGORY_SOCIAL" | "social" => Some("social"),
        "CATEGORY_FORUMS" | "forums" => Some("forums"),
        "CATEGORY_NOTIFICATIONS" | "notifications" => Some("notifications"),
        _ => None,
    }
}

fn row_label_names(row: &JsValue) -> Vec<String> {
    let labels = js::get(row, "labelNames");
    let mut out: Vec<String> = Vec::new();
    // `labelNames` is an array of **strings** — `arr[i].name` on a string is
    // undefined, which is the trap `effLabels` already sprang once.
    if let Some(a) = labels.dyn_ref::<Array>() {
        let mut cats: Vec<String> = Vec::new();
        for i in 0..a.length() {
            let Some(name) = a.get(i).as_string() else { continue };
            if let Some(cat) = category_name(&name) {
                cats.push(cat.to_string());
                continue;
            }
            if HIDDEN_LABELS.contains(&name.as_str()) {
                continue;
            }
            out.push(name);
        }
        out.extend(cats);
    }
    out.truncate(3);
    out
}

/// `lab-<slug>` picks the colour, `user` / `cat` / `bad` the shape: a label you
/// made, a category Gmail made, and a folder that means "this is gone".
fn label_chip_class(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let kind = match name {
        "SPAM" | "TRASH" => " bad",
        _ if category_name(name).is_some() => " cat",
        _ => " user",
    };
    format!("lab-{slug}{kind}")
}

/// A stable hue for a label a person made: FNV-1a over the name, into a curated
/// set of hues (a hash straight into 0..360 produces muddy neighbours). Kept
/// away from the category hues styles.css already uses so the two groups never
/// look like each other.
fn label_style(name: &str) -> String {
    if category_name(name).is_some() {
        return String::new();
    }
    if HIDDEN_LABELS.contains(&name) || name == "SPAM" || name == "TRASH" {
        return String::new();
    }
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h = (h ^ *b as u32).wrapping_mul(16777619);
    }
    const HUES: [u32; 10] = [12, 38, 62, 100, 150, 190, 225, 265, 305, 340];
    let hue = HUES[(h as usize) % HUES.len()];
    format!(
        "color:hsl({hue} 85% 80%);border-color:hsla({hue},70%,62%,.45);background:hsla({hue},70%,55%,.14)"
    )
}

/// `45s · 12m · 4h · 3d · 4mo · 1y` out of an epoch-ms timestamp.
fn compact_time(ms: f64) -> String {
    if ms <= 0.0 {
        return "—".to_string();
    }
    let secs = ((Date::now() - ms) / 1000.0).max(0.0);
    if secs < 60.0 {
        return format!("{}s", secs.round() as i64);
    }
    let mins = (secs / 60.0).round() as i64;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = (secs / 3600.0).round() as i64;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = (secs / 86400.0).round() as i64;
    if days < 30 {
        return format!("{days}d");
    }
    let months = (secs / (86400.0 * 30.0)).round() as i64;
    if months < 12 {
        return format!("{months}mo");
    }
    format!("{}y", (months as f64 / 12.0).round() as i64)
}

fn topbar() -> Node {
    m("header")
        .class("mail-top")
        .kids(vec![
            m("div").class("brand").kids(vec![
                Node::icon("envelope-open"),
                m("b").child(lit("mail1")),
                m("span").class("tag").child(lit("rust backend · sqlite · gmail")),
            ]),
            m("div").class("grow"),
            m("span").class("sync").kids(vec![
                m("i").class("ph ph-cloud-arrow-down"),
                m("span").child(lit("pull:")),
                m("code").text("$store.mail.lastPull.listed"),
                m("span").child(lit(" ids · ")),
                m("code").text("$store.mail.lastPull.totalMs"),
                m("code").child(lit("ms")),
                m("span").class("muted").child(lit(" · ")),
                m("span").text("$store.mail.lastPullAgo"),
                // Oldest-first backfill: how many Primary inbox ids are still
                // unread into the mirror. Hidden once the mirror is caught up.
                m("span")
                    .class("muted")
                    .show("$store.mail.lastPull.backlog > 0")
                    .text("' · backfill ' + $store.mail.lastPull.backlog + ' to go'"),
            ]),
            m("span").class("sync").kids(vec![
                m("i").class("ph ph-cloud-arrow-up"),
                m("span").child(lit("push:")),
                m("code").text("$store.mail.lastPush.applied"),
                m("span").class("muted").child(lit(" · ")),
                m("span").text("$store.mail.lastPushAgo"),
            ]),
            m("span").class("sync").kids(vec![
                m("i").class("ph ph-tray"),
                m("code").text("$store.mail.counts.inbox"),
                m("span").child(lit("in · ")),
                m("code").text("$store.mail.counts.unread"),
                m("span").child(lit("unread · ")),
                m("code").text("$store.mail.counts.total"),
                m("span").child(lit("total")),
            ]),
            // A pull that stopped early used to look exactly like one that
            // finished. `quota` is Gmail's 403 backoff, `errors` its reasons.
            m("span")
                .class("warn-flag")
                .show("$store.mail.lastPull.quota")
                .attr("title", "Gmail returned 403 PERMISSION_DENIED / quota exceeded")
                .kids(vec![
                    m("i").class("ph ph-gauge"),
                    m("span").child(lit("quota backoff")),
                ]),
            m("span")
                .class("warn-flag")
                .show("$store.mail.lastPull.errors.length > 0")
                .bind("title", "$store.mail.lastPull.errors[0]")
                .kids(vec![
                    m("i").class("ph ph-warning-octagon"),
                    m("span").text("$store.mail.lastPull.errors.length"),
                    m("span").child(lit(" pull error(s)")),
                ]),
        ])
}

fn search_bar() -> Node {
    m("div").class("mail-search").kids(vec![
        m("i").class("ph ph-magnifying-glass"),
        m("input")
            .attr("type", "text")
            .attr("data-search", "1")
            .attr("placeholder", "Search by subject, from or snippet…")
            .class("form-input")
            .model("$store.mail.q")
            // `@input` is **taken**: m-js registers x-model as `attrs.oninput`, and
            // an `@input` handler writes that same key, so binding one here
            // silently replaced the model and `q` never changed. `keyup` fires
            // just after `input`, so the store already holds the new text.
            .on("keyup", "$store.mail.searchDebounced()")
            .on("keydown.enter", "$store.mail.searchNow()"),
        m("button")
            .attr("type", "button")
            .class("clear")
            .show("$store.mail.q !== ''")
            .on("click", "$store.mail.clearFilters()"),
        m("div").class("grow"),
        m("div").class("segmented filters").kids(vec![
            filter_btn("inbox", "Inbox", "$store.mail.counts.inbox", "inbox messages in the mirror"),
            filter_btn("unread", "Unread", "$store.mail.counts.unread", "unread in the mirror"),
            filter_btn("starred", "Starred", "$store.mail.counts.starred", "starred in the mirror"),
            filter_btn("pending", "Queued", "$store.mail.queuedCount", "rows with a queued action"),
            // `counts.total` is the whole mirror, not the list on screen — the
            // readout at the right of this bar is what the list actually holds.
            filter_btn("", "All", "$store.mail.counts.total", "every message in the mirror"),
        ]),
        m("button")
            .attr("type", "button")
            .class("refresh")
            .attr("title", "reload the list from SQLite")
            .on("click", "$store.mail.refresh()"),
        // "1000 of 1243": the window the list holds, and the mirror's own answer
        // for how many rows the filter + search match. Without the second number
        // a truncated search looks complete.
        m("span")
            .class("tiny muted window-count")
            .attr("title", "rows on screen / rows matching in the whole mirror")
            .kids(vec![
                m("b").text("$store.mail.rows.length"),
                m("span")
                    .class("of-total")
                    .show("$store.mail.matched > $store.mail.rows.length")
                    .kids(vec![
                        m("span").child(lit(" of ")),
                        m("b").text("$store.mail.matched"),
                    ]),
            ]),
    ])
}

fn filter_btn(value: &str, label: &str, count: &str, title: &str) -> Node {
    m("button")
        .attr("type", "button")
        .class("seg")
        .attr("title", title)
        .bind("class", &format!("filterClass('{}')", value))
        .on("click", &format!("$store.mail.setFilter('{}')", value))
        .kids(vec![
            m("span").child(lit(label)),
            // The counts the list already knows: a chip you cannot read the
            // size of is a chip you have to click to find out about.
            m("b").class("seg-count").text(count),
        ])
}

fn tool_btn(icon: &str, label: &str, click: &str) -> Node {
    m("button")
        .attr("type", "button")
        .class("toolbtn")
        .attr("title", label)
        .on("click", click)
        .kids(vec![Node::icon(icon), m("span").child(lit(label))])
}

fn tool_bar() -> Node {
    m("div").class("mail-toolbar").kids(vec![
        m("button")
            .attr("type", "button")
            .class("select-all")
            .attr("title", "select / clear the selection")
            .bind("class", "selAllClass()")
            .on("click", "$store.mail.toggleSelectAll()")
            .kids(vec![
                m("i").class("ph ph-check-square"),
                m("i").class("ph ph-square"),
            ]),
        m("div").class("div"),
        tool_btn("package", "Archive", "$store.mail.act('archive')"),
        tool_btn("trash", "Delete", "$store.mail.act('delete')"),
        tool_btn("prohibit", "Spam", "$store.mail.act('spam')"),
        tool_btn("envelope-open", "Read", "$store.mail.act('read')"),
        tool_btn("envelope", "Unread", "$store.mail.act('unread')"),
        tool_btn("folder", "Move…", "$store.ui.openAction('move')"),
        tool_btn("x-circle", "Purge…", "$store.ui.openAction('purge')"),
        m("div").class("div"),
        tool_btn("star", "Star", "$store.mail.act('star')"),
        // The rules button: run the disk rules over the target set, ask the model,
        // and put a recommendation on each row. It never acts on its own.
        m("button")
            .attr("type", "button")
            .class("toolbtn rules")
            .attr("title", "run rules/*.yaml over the selected emails and ask the model what each deserves")
            .bind("disabled", "$store.mail.recBusy")
            .on("click", "$store.mail.evaluateRules()")
            .kids(vec![
                Node::icon("magic-wand"),
                m("span").child(lit("Rules")),
                m("b")
                    .class("rec-count")
                    .show("$store.mail.ruleCount > 0")
                    .text("$store.mail.ruleCount"),
            ]),
        // New Rule: narrate what a rule should do with the selected emails and
        // the text LLM writes a rules/*.yaml file. Needs a text model on :1234.
        m("button")
            .attr("type", "button")
            .class("toolbtn newrule")
            .attr("title", "draft a new rules/*.yaml from a description of the selected emails (needs a text model on :1234)")
            .on("click", "$store.mail.openNewRule()")
            .kids(vec![
                Node::icon("plus-circle"),
                m("span").child(lit("New Rule")),
            ]),
        m("button")
            .attr("type", "button")
            .class("clear")
            .attr("title", "drop the recommendations on screen")
            .show("$store.mail.recCount > 0")
            .on("click", "$store.mail.clearRecs()"),
        // The phraser button: run phrasers/*.yaml over the target set and ask
        // the text LLM to reword each match. The two-model workflow means the
        // System One model is usually running instead — stop it, start a text
        // model on :1234, press this, swap back.
        m("button")
            .attr("type", "button")
            .class("toolbtn phrase")
            .attr("title", "run phrasers/*.yaml over the selected emails — a text LLM rewords each match (needs a text model on :1234)")
            .bind("disabled", "$store.mail.phraseBusy")
            .on("click", "$store.mail.phraseRows()")
            .kids(vec![
                Node::icon("quotes"),
                m("span").child(lit("Phrase")),
            ]),
        m("button")
            .attr("type", "button")
            .class("clear")
            .attr("title", "drop every stored rewording — rows show their own words again")
            .show("anyPhrased()")
            .on("click", "$store.mail.clearPhrases()"),
        m("div").class("grow"),
        // `submit()` writes this line; nothing used to render it.
        m("span")
            .class("flash")
            .show("$store.ui.flash !== ''")
            .text("$store.ui.flash"),
        // Slice in the template. A new store getter is copied once on hot
        // reload and then never updates, which left this pager at 1 / 1 and
        // the list empty.
        m("div").class("pager").kids(vec![
            m("button")
                .attr("type", "button")
                .class("page-btn")
                .attr("title", "previous page")
                .bind("disabled", "$store.mail.page === 0")
                .on("click", "$store.mail.pagePrev()")
                .child(lit("<")),
            m("span").class("page-label").text("($store.mail.page + 1) + ' / ' + Math.max(1, Math.ceil($store.mail.visible.length / 20))"),
            m("button")
                .attr("type", "button")
                .class("page-btn")
                .attr("title", "next page")
                .bind("disabled", "$store.mail.page + 1 >= Math.max(1, Math.ceil($store.mail.visible.length / 20))")
                .on("click", "$store.mail.pageNext()")
                .child(lit(">")),
        ]),
    ])
}

fn action_bar() -> Node {
    m("div")
        .class("mail-actionbar")
        .show("$store.ui.open")
        .kids(vec![
            m("div").class("mail-actionbar-row").kids(vec![
                m("span").class("action-label").text("$store.ui.action"),
                m("span").class("muted").child(lit("   ")),
                m("span").class("action-target").text("$store.ui.targetSummary"),
            ]),
            m("div").class("mail-actionbar-row fields").kids(vec![
                m("select")
                    .attr("data-dest", "1")
                    .model("$store.ui.destination")
                    .kids(vec![
                        m("option").attr("value", "").child(lit("Choose a folder…")),
                        m("option")
                            .for_each("f in $store.mail.folders")
                            .key("f")
                            .bind("value", "f")
                            .text("f"),
                    ]),
                m("input")
                    .attr("placeholder", "instruction (optional)")
                    .class("form-input")
                    .model("$store.ui.instruction"),
                m("input")
                    .attr("placeholder", "rationale (optional)")
                    .class("form-input")
                    .model("$store.ui.rationale"),
            ]),
            m("div").class("mail-actionbar-row end").kids(vec![
                m("span").class("muted").child(lit("targets:")),
                m("span").text("$store.ui.targetSummary"),
                m("div").class("grow"),
                m("button")
                    .attr("type", "button")
                    .class("submit")
                    .bind("disabled", "$store.ui.submitting")
                    .on("click", "$store.ui.submit()"),
                m("button")
                    .attr("type", "button")
                    .class("cancel")
                    .on("click", "$store.ui.closeAction()"),
                // "Delete" is Gmail's Trash — recoverable. "Purge" is a real
                // delete, so it says so before you press submit.
                m("span")
                    .class("error")
                    .show("$store.ui.action === 'purge'")
                    .child(lit("purge = permanent, not Trash")),
                m("span").class("error").show("$store.ui.err").text("$store.ui.err"),
            ]),
        ])
}

/// The New Rule dialog: a narration box over the selected emails. Submitting
/// asks the text LLM to draft a `rules/*.yaml` file from the narration and the
/// example emails.
fn new_rule_dialog() -> Node {
    m("div")
        .class("mail-actionbar new-rule")
        .show("$store.mail.ruleOpen")
        .kids(vec![
            m("div").class("mail-actionbar-row").kids(vec![
                m("span").class("action-label").child(lit("New rule")),
                m("span").class("muted").text("ruleScope()"),
            ]),
            m("div").class("mail-actionbar-row fields").kids(vec![
                m("input")
                    .attr("type", "text")
                    .class("form-input rule-prompt")
                    .attr("placeholder", "What should happen to emails like these? e.g. move Chase purchase alerts to Expenses")
                    .model("$store.mail.rulePrompt")
                    .on("keydown.enter", "$store.mail.submitNewRule(ruleNarration())"),
            ]),
            m("div").class("mail-actionbar-row end").kids(vec![
                m("button")
                    .attr("type", "button")
                    .class("submit")
                    .bind("disabled", "$store.mail.ruleBusy")
                    .on("click", "$store.mail.submitNewRule(ruleNarration())")
                    .child(lit("Create rule")),
                m("button")
                    .attr("type", "button")
                    .class("cancel")
                    .on("click", "$store.mail.closeNewRule()")
                    .child(lit("Cancel")),
                m("span")
                    .class("muted")
                    .show("$store.mail.ruleBusy")
                    .child(lit("drafting\u{2026}")),
                m("span")
                    .class("error")
                    .show("$store.mail.ruleErr !== ''")
                    .text("$store.mail.ruleErr"),
            ]),
        ])
}

/// Pull / push, and the knobs the pull takes. The two buttons the keyboard
/// shortcuts (`p`, `u`) mirror — a first-time visitor should not have to know
/// the keymap exists.
fn controls_row() -> Node {
    m("div").class("mail-controls").kids(vec![
        m("button")
            .attr("type", "button")
            .class("btn pull")
            .attr("title", "Gmail → SQLite (key p)")
            .bind("disabled", "$store.mail.pulling")
            .on("click", "$store.mail.pull()")
            .kids(vec![
                m("i").class("ph ph-cloud-arrow-down"),
                m("span").child(lit("Pull")),
                m("kbd").child(lit("p")),
            ]),
        m("button")
            .attr("type", "button")
            .class("btn push")
            .attr("title", "queued mutations → Gmail (key u)")
            .bind("disabled", "$store.mail.pushing || $store.mail.queuedCount === 0")
            .on("click", "$store.mail.push()")
            .kids(vec![
                m("i").class("ph ph-cloud-arrow-up"),
                m("span").child(lit("Push")),
                m("kbd").child(lit("u")),
                m("span")
                    .class("queued-badges")
                    .show("$store.mail.queuedCount > 0")
                    .text("$store.mail.queuedCount"),
            ]),
        m("span").class("tiny muted").child(lit("queued:")),
        m("b").class("queued").text("$store.mail.queuedCount"),
        m("button")
            .attr("type", "button")
            .class("clear-queue")
            .attr("title", "drop every queued action")
            .show("$store.mail.queuedCount > 0")
            .on("click", "$store.mail.cancelAll()"),
        m("div").class("grow"),
        m("label").kids(vec![
            m("span").child(lit("query")),
            m("select")
                .attr("data-query", "1")
                .model("$store.mail.query")
                .kids(vec![
                    m("option").attr("value", "in:inbox").child(lit("Inbox")),
                    m("option")
                        .attr("value", "in:inbox is:unread")
                        .child(lit("Unread only")),
                    m("option")
                        .attr("value", "-in:(spam OR trash) -in:draft")
                        .child(lit("All mail")),
                ]),
        ]),
        m("label").kids(vec![
            m("span").child(lit("max")),
            m("input")
                .attr("type", "number")
                .attr("min", "1")
                .attr("step", "100")
                .class("num")
                .model("$store.mail.max"),
        ]),
        m("label").kids(vec![
            m("span").child(lit("req/s")),
            m("input")
                .attr("type", "number")
                .attr("min", "1")
                .attr("max", "60")
                .attr("step", "1")
                .class("num")
                .model("$store.mail.rate"),
        ]),
        m("label").kids(vec![
            m("input")
                .attr("type", "checkbox")
                .model("$store.mail.fullBodies")
                .attr("title", "fetch every body during the pull instead of on open"),
            m("span").child(lit("full bodies")),
        ]),
    ])
}

/// The keymap, spelled out where the user is looking at the list.
fn hint_strip() -> Node {
    m("div").class("hint").kids(vec![
        hint("j / k", "up / down"),
        hint("l / ;", "page"),
        hint("space", "select"),
        hint("click / enter", "open"),
        hint("a", "select all / none"),
        hint("q", "read / unread"),
        hint("r", "rules"),
        hint("z", "apply rec"),
        hint("y", "star"),
        hint("p / u", "pull / push"),
        hint("/", "search"),
        hint("esc", "back out"),
    ])
}

fn hint(key: &str, what: &str) -> Node {
    m("span").kids(vec![m("kbd").child(lit(key)), m("span").child(lit(what))])
}

fn list_region() -> Node {
    m("main")
        .class("mail-list")
        .show("!$store.mail.openId")
        .kids(vec![
            m("div")
                .class("empty")
                .show("$store.mail.visible.length === 0 && !$store.mail.loading")
                .kids(vec![
                    m("div").class("big-icon").child(lit("📭")),
                    m("h2").child(lit("Inbox zero!")),
                    m("p").child(lit("Press PULL (key p) to bring in mail.")),
                ]),
            m("ul").class("mail-rows").kids(vec![
                m("li")
                    .class("mail-row")
                    .for_each("r in $store.mail.visible.slice($store.mail.page * 20, ($store.mail.page + 1) * 20)")
                    .key("r.id")
                    .bind("data-id", "r.id")
                    .bind("class", "rowClass(r)")
                    .on("click", "rowClick(r, $event)")
                    .kids(vec![
                        m("button")
                            .attr("type", "button")
                            .class("sel-check")
                            .bind("class", "checkClass(r)")
                            .on("click.stop", "sel(r, $event)")
                            .kids(vec![
                                m("i").class("ph ph-check-square"),
                                m("i").class("ph ph-square"),
                            ]),
                        m("button")
                            .attr("type", "button")
                            .class("star-btn")
                            .bind("class", "starClass(r)")
                            .on("click.stop", "$store.mail.toggleRow(r.id, 'star')")
                            .kids(vec![Node::icon("star")]),
                        m("button")
                            .attr("type", "button")
                            .class("imp-btn")
                            .attr("title", "important")
                            .bind("class", "impClass(r)")
                            .on("click.stop", "$store.mail.toggleRow(r.id, 'important')")
                            .kids(vec![Node::icon_dyn("impIcon(r)")]),
                        // One column, four phases of the same button:
                        // Rules → (no match) New Rule → (match) recommendation → Undo → Rules.
                        m("button")
                            .attr("type", "button")
                            .class("toolbtn rules row-fsm")
                            .attr("title", "run the rules on this email")
                            .show("phaseRules(r.id)")
                            .bind("disabled", "$store.mail.recBusy")
                            .on("click.stop", "$store.mail.evaluateRulesRow(r.id)")
                            .kids(vec![Node::icon("magic-wand"), m("span").child(lit("Rules"))]),
                        m("button")
                            .attr("type", "button")
                            .class("toolbtn newrule row-fsm")
                            .attr("title", "write a rule for this email")
                            .show("phaseNew(r.id)")
                            .on("click.stop", "$store.mail.openNewRule(r.id)")
                            .kids(vec![Node::icon("plus-circle"), m("span").child(lit("New Rule"))]),
                        m("button")
                            .attr("type", "button")
                            .class("rec-btn")
                            .show("phaseRec(r.id)")
                            .bind("class", "recClass(r.id)")
                            .bind("title", "recTitle(r.id)")
                            .on("click.stop", "$store.mail.applyRec(r.id)")
                            .kids(vec![
                                Node::icon("magic-wand"),
                                m("span").text("recShort(r.id)"),
                            ]),
                        m("span")
                            .class("skipped")
                            .show("phaseSkip(r.id)")
                            .bind("title", "recTitle(r.id)")
                            .child(lit("Skipped.")),
                        m("button")
                            .attr("type", "button")
                            .class("toolbtn row-fsm undo")
                            .attr("title", "take the queued action back — this email returns to Rules")
                            .show("phaseUndo(r.id)")
                            .on("click.stop", "$store.mail.undoRow(r.id)")
                            .kids(vec![Node::icon("arrow-counter-clockwise"), m("span").child(lit("Undo"))]),
                        m("span").class("sender").text("r.fromName"),
                        m("div").class("summary").kids(vec![
                            // A reworded email shows the phraser's words, marked
                            // with the quotes glyph and carrying the original in
                            // its title. The row keeps its shape either way.
                            m("i").class("ph ph-quotes phrased-mark").show("isPhrased(r)"),
                            m("span")
                                .class("subject")
                                .bind("title", "subjTitle(r)")
                                .text("subjText(r)"),
                            m("span").class("snippet").text("sumText(r)"),
                        ]),
                        m("div").class("row-labels").kids(vec![
                            m("span")
                                .class("label-chip")
                                .for_each("l in rowLabels(r)")
                                .key("l")
                                .bind("class", "labelChipClass(l)")
                                .bind("style", "labelStyle(l)")
                                .text("l"),
                        ]),
                        m("div").class("pending-cell").kids(vec![
                            m("span")
                                .class("pending-chip")
                                .show("r.pendingCount > 0")
                                .text("pendingChip(r)"),
                            // The queue had one undo for the whole list. This is
                            // the one for a single row.
                            m("button")
                                .attr("type", "button")
                                .class("cancel-one")
                                .attr("title", "cancel the queued action on this email")
                                .show("r.pendingCount > 0")
                                .on("click.stop", "$store.mail.cancelQueue(r.id)")
                                .kids(vec![Node::icon("x-circle")]),
                        ]),
                        m("span")
                            .class("time")
                            .bind("title", "timeTitle(r)")
                            .kids(vec![
                                // Not `ph-clock-arrow-ccw`: that name is not in
                                // the Phosphor build the kit ships, and an
                                // unknown `ph-*` renders as nothing at all.
                                m("i").class("ph ph-clock-counter-clockwise"),
                                m("span").text("timeText(r)"),
                            ]),
                    ]),
            ]),
        ])
}

fn detail_region() -> Node {
    m("main")
        .class("mail-detail")
        .show("$store.mail.openId")
        .kids(vec![
            m("div").class("detail-head").kids(vec![
                m("button")
                    .attr("type", "button")
                    .class("back")
                    .on("click", "$store.mail.closeMail()")
                    .kids(vec![
                        Node::icon("arrow-left"),
                        m("span").child(lit("back")),
                    ]),
                m("div").class("grow"),
                m("div").class("mode-toggle").kids(vec![
                    m("button")
                        .attr("type", "button")
                        .class("mode")
                        .bind("class", "modeClass('html')")
                        .on("click", "$store.mail.setOpenMode('html')")
                        .child(lit("html")),
                    m("button")
                        .attr("type", "button")
                        .class("mode")
                        .bind("class", "modeClass('text')")
                        .on("click", "$store.mail.setOpenMode('text')")
                        .child(lit("text")),
                ]),
            ]),
            m("div").class("detail-body").kids(vec![
                m("div").class("detail-top").kids(vec![
                    m("div").class("sender").kids(vec![
                        m("b").text("$store.mail.open.fromName"),
                        m("span").class("tiny muted").text(" $store.mail.open.fromEmail"),
                    ]),
                    m("div").class("subject").text("openSubject()"),
                ]),
                m("div").class("meta").kids(vec![
                    m("span").class("tiny muted").child(lit("id ")),
                    m("code").text("$store.mail.open.id"),
                    m("span").class("tiny muted").child(lit("  thread ")),
                    m("code").text("$store.mail.open.threadId"),
                    m("span").class("tiny muted").child(lit("  · ")),
                    m("code").text("$store.mail.open.dateIso"),
                    m("span").class("tiny muted").child(lit("  · size ")),
                    m("code").text("$store.mail.open.size"),
                ]),
                m("div").class("labels").kids(vec![
                    m("i").class("ph ph-tag"),
                    m("span")
                        .for_each("l in $store.mail.open.labelNames")
                        .key("l")
                        .class("label-chip")
                        .bind("class", "labelChipClass(l)")
                        .bind("style", "labelStyle(l)")
                        .kids(vec![m("span").text("l")]),
                ]),
                m("div").class("pending").kids(vec![
                    m("span").class("tiny muted").child(lit("queued:")),
                    m("ol").kids(vec![
                        m("li")
                            .for_each("p, i in $store.mail.open.pending")
                            .key("i")
                            .kids(vec![
                                m("code").text("pendingOp(p)"),
                                m("span").child(lit(" · ")),
                                m("span").text("pendingNames(p)"),
                            ]),
                    ]),
                    m("button")
                        .attr("type", "button")
                        .class("cancel-one wide")
                        .attr("title", "cancel every queued action on this email")
                        .show("$store.mail.open.pendingCount > 0")
                        .on("click", "$store.mail.cancelQueue($store.mail.open.id)")
                        .kids(vec![
                            Node::icon("x-circle"),
                            m("span").child(lit("cancel queued")),
                        ]),
                ]),
                // What the rules + the model say about *this* email, with each
                // rule's own answer listed. The recommendation is shown, then
                // pressed — it is never applied on the user's behalf.
                m("div")
                    .class("rec-detail")
                    .show("hasRec($store.mail.open.id)")
                    .kids(vec![
                        m("div").class("rec-line").kids(vec![
                            m("span").class("tiny muted").child(lit("rules recommend:")),
                            m("button")
                                .attr("type", "button")
                                .class("rec-btn")
                                .show("phaseRec($store.mail.open.id)")
                                .bind("class", "recClass($store.mail.open.id)")
                                .bind("title", "recTitle($store.mail.open.id)")
                                .on("click", "$store.mail.applyRec($store.mail.open.id)")
                                .kids(vec![
                                    Node::icon("magic-wand"),
                                    m("span").text("recShort($store.mail.open.id)"),
                                ]),
                            m("span")
                                .class("skipped")
                                .show("phaseSkip($store.mail.open.id)")
                                .bind("title", "recTitle($store.mail.open.id)")
                                .child(lit("Skipped.")),
                            m("button")
                                .attr("type", "button")
                                .class("toolbtn row-fsm undo")
                                .show("phaseUndo($store.mail.open.id)")
                                .attr("title", "take the queued action back")
                                .on("click", "$store.mail.undoRow($store.mail.open.id)")
                                .kids(vec![
                                    Node::icon("arrow-counter-clockwise"),
                                    m("span").child(lit("Undo")),
                                ]),
                        ]),
                        m("ol").kids(vec![
                            m("li")
                                .for_each("q in recRuleNames($store.mail.open.id)")
                                .key("q")
                                .kids(vec![m("code").text("q")]),
                        ]),
                    ]),
                m("div").class("body-wrap").kids(vec![
                    m("div")
                        .class("body-missing")
                        .show("!$store.mail.open.hasBody")
                        .kids(vec![
                            m("p").child(lit("No body stored yet. The pull only kept the headers.")),
                            m("button")
                                .attr("type", "button")
                                .class("btn pull")
                                .bind("disabled", "$store.mail.openLoading")
                                .on("click", "$store.mail.loadBody()")
                                .child(lit("Load full body")),
                        ]),
                    m("iframe")
                        .class("raw-html")
                        // allow-same-origin so the app can catch the mouse back
                        // button inside the preview. Scripts stay blocked, so the
                        // mail itself still cannot run. `about:blank` plus a
                        // document.write — not `srcdoc` — so the preview is not
                        // its own Back step.
                        .attr("sandbox", "allow-same-origin")
                        .attr("referrerpolicy", "no-referrer")
                        .attr("src", "about:blank")
                        .show("$store.mail.open.hasBody && $store.mail.open.bodyKind === 'html'"),
                    m("pre")
                        .class("raw-text")
                        .show("$store.mail.open.hasBody && $store.mail.open.bodyKind !== 'html'")
                        .text("$store.mail.open.bodyText"),
                ]),
            ]),
        ])
}
