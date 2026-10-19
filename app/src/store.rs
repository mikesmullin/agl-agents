//! The app's state, and its conversation with the SQLite-backed API.
//!
//! Two stores, both on `window` via `M.store()`, so a hot swap keeps what the
//! user was looking at:
//!
//! * `mail` — the mirror: rows, counts, labels, the filter/search, the
//!   selection, the open message, and the pull/push reports
//! * `ui`   — the progressive-disclosure action bar: which action is open, its
//!   form (destination / instruction / rationale), and the flash/error line
//!
//! ## Where the work lives
//!
//! Every action is a method on the **store**, not on the page. A route is then
//! just markup, the two routes share one behaviour, and a hot reload cannot
//! lose a half-typed instruction.
//!
//! ## The write path, and why the UI changes before Gmail does
//!
//! ```text
//! click Archive → POST /api/queue  (SQLite only, no network to Gmail)
//!               → response carries the affected rows, already carrying their
//!                 pending mutations and their *effective* labels
//!               → those rows replace the ones in `mail.rows` → repaint now
//! later, PUSH  → POST /api/push → Gmail → the mirror is written from the
//!                 accepted result
//! ```
//!
//! So "queued, rendered immediately, pushed later" is not a trick in the
//! template: the server computes `effLabels = labels + pending.add − pending.remove`
//! and the row shows that pending state (the chip, Undo). The Inbox list itself
//! keeps using the stored `inInbox` flag until Push, so a queued move does not
//! vanish before it has been sent.

use std::cell::{Cell, RefCell};

use js_sys::{Array, Date, Object};
use wasm_bindgen::prelude::*;

use rustex1_kit::api;
use rustex1_kit::js;
use rustex1_kit::mjs;
use rustex1_kit::mjs::Def;
use rustex1_kit::session;

pub fn register() {
    mail();
    ui();
}

// ---------------------------------------------------------------------------
// mail
// ---------------------------------------------------------------------------

fn mail() {
    let mut d = Def::new().named("mail store");
    d.val("rows", &Array::new().into())
        .val("counts", &Object::new().into())
        // How many rows in the **whole mirror** match the current filter + q,
        // as opposed to `rows.length`, which is only the fetched window.
        .num("matched", 0.0)
        .val("labels", &Array::new().into())
        .str("filter", "inbox")
        .str("q", "")
        .str("query", "in:inbox")
        .num("max", 1000.0)
        .num("rate", 25.0)
        .bool("fullBodies", false)
        .bool("loading", false)
        .bool("pulling", false)
        .bool("pushing", false)
        .val("lastPull", &Object::new().into())
        .val("lastPush", &Object::new().into())
        .val("recs", &Object::new().into())
        .num("recCount", 0.0)
        // Ids whose recommendation was just queued. The row button shows Undo
        // for these, and Undo returns the row to the untouched Rules state.
        .val("undoIds", &Object::new().into())
        // Set when New Rule was opened from one row, so submit targets that
        // email instead of the selection.
        .str("ruleRow", "")
        .str("ruleErr", "")
        .num("ruleCount", 0.0)
        .bool("recBusy", false)
        .bool("phraseBusy", false)
        // The New Rule dialog: open, the narration being typed, and whether a
        // draft is in flight.
        .bool("ruleOpen", false)
        .str("rulePrompt", "")
        .bool("ruleBusy", false)
        .val("selected", &Array::new().into())
        .str("focusId", "")
        // The checkbox last clicked without Shift. Shift-click selects every
        // visible row from here to the box just clicked.
        .str("anchorId", "")
        // 0-based page of the filtered list. Page size is `PAGE_SIZE`.
        .num("page", 0.0)
        .str("openId", "")
        .val("open", &Object::new().into())
        .bool("openLoading", false)
        .str("openMode", "html")
        .str("error", "");

    // ---- derived views, computed in Rust ----------------------------------
    d.getter("visible", |this, _| filtered_rows(&this).into());

    d.getter("selectedCount", |this, _| {
        JsValue::from(selected_ids(&this).len() as f64)
    });

    d.getter("queuedCount", |this, _| {
        let counts = js::get(&this, "counts");
        JsValue::from(js::get_num(&counts, "queued"))
    });

    // The Move dropdown: the folders **the user** made. Gmail's own labels
    // (INBOX, SPAM, TRASH, CATEGORY_*, the coloured stars, …) all carry their
    // reserved ids; every label a person created has an id like `Label_12`.
    // `kind` is only Gmail's *visibility* setting (labelShow / labelHide /
    // labelShowIfUnread), so it cannot tell the two groups apart.
    d.getter("folders", |this, _| {
        let labels = js::get(&this, "labels");
        let mut names: Vec<String> = Vec::new();
        if let Some(list) = labels.dyn_ref::<Array>() {
            for i in 0..list.length() {
                let l = list.get(i);
                let id = js::get_str(&l, "id");
                let name = js::get_str(&l, "name");
                if !id.starts_with("Label_") || name.is_empty() {
                    continue;
                }
                names.push(name);
            }
        }
        names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        let out = Array::new();
        for name in names {
            out.push(&JsValue::from_str(&name));
        }
        out.into()
    });

    // Rows the toolbar actions will act on: the open email (while the detail
    // is up), else the selection, else the focused row.
    d.getter("targetIds", |this, _| {
        let ids = target_ids(&this);
        js::array_of(&ids.iter().map(|s| s.as_str()).collect::<Vec<_>>()).into()
    });

    // "3m ago" for the status strip. Recomputed by the 4 s poll, which is the
    // coarsest clock the strip needs — `ui-reltime` is the widget for a ticking
    // one, and these two are summary text, not a timestamp you read.
    d.getter("lastPullAgo", |this, _| ago(&js::get(&this, "lastPull")));
    d.getter("lastPushAgo", |this, _| ago(&js::get(&this, "lastPush")));

    // "Is every row **on screen** selected?" — not every row in the fetched
    // window. With a filter on, half the window is hidden, and comparing the
    // selection against `rows` made this answer "no" forever.
    d.getter("allSelected", |this, _| {
        let list = page_rows(&this);
        if list.length() == 0 {
            return JsValue::FALSE;
        }
        for i in 0..list.length() {
            if !is_selected(&this, &js::get_str(&list.get(i), "id")) {
                return JsValue::FALSE;
            }
        }
        JsValue::TRUE
    });

    // ---- methods ----------------------------------------------------------
    // A filter is a *server* query, not just a client-side predicate: the
    // mirror holds 5000 messages and `rows` only ever carries the current
    // window, so switching to "starred" has to re-fetch or it filters a list
    // that was fetched for "inbox".
    d.action("setFilter", |this, args| {
        js::set_str(&this, "filter", &js::arg_str(&args, 0));
        js::set_num(&this, "page", 0.0);
        bump_search();
        poll();
    });
    d.action("pagePrev", |this, _args| {
        let total = filtered_rows(&this).length() as usize;
        let page = page_index(&this, total);
        if page > 0 {
            js::set_num(&this, "page", (page - 1) as f64);
        }
    });
    d.action("pageNext", |this, _args| {
        let total = filtered_rows(&this).length() as usize;
        let page = page_index(&this, total);
        if page + 1 < page_count(total) {
            js::set_num(&this, "page", (page + 1) as f64);
        }
    });
    d.action("clearFilters", |this, _args| {
        js::set_str(&this, "filter", "inbox");
        js::set_str(&this, "q", "");
        js::set_num(&this, "page", 0.0);
        bump_search();
        poll();
    });
    // `q` is a *server* predicate too — `/api/mail?q=` LIKEs subject / from /
    // snippet across all 5000 rows — but nothing re-fetched when it changed, so
    // a match outside the fetched window was unreachable: the client-side
    // predicate could only ever hide rows the server had already sent.
    //
    // Debounced by generation: every keystroke bumps the counter and only the
    // newest timer is allowed to fetch. `lifecycle::after` is one-shot per call,
    // so a stale timer simply fails its generation check and does nothing.
    d.action("searchDebounced", |this, _args| {
        js::set_num(&this, "page", 0.0);
        let q = js::get_str(&this, "q");
        let gen = bump_search();
        rustex1_kit::lifecycle::after(400.0, move || {
            if SEARCH_GEN.with(|g| g.get()) != gen {
                return; // a newer keystroke owns the refresh
            }
            let m = mjs::store_ref("mail");
            if js::is_missing(&m) {
                return;
            }
            if js::get_str(&m, "q") != q {
                return;
            }
            // The 4 s tick may already have fetched this exact query.
            if fetch_key(&m) == LAST_FETCH.with(|k| k.borrow().clone()) {
                return;
            }
            poll();
        });
    });
    // Enter in the search box: fetch now, do not wait out the debounce.
    d.action("searchNow", |_this, _args| {
        bump_search();
        poll();
    });
    // esc drops the search. The rows a search hid are hidden by the **server's**
    // WHERE, not by a class, so clearing `q` in the store is not enough — the
    // list has to come back from SQLite.
    d.action("clearSearch", |this, _args| {
        js::set_str(&this, "q", "");
        js::set_num(&this, "page", 0.0);
        bump_search();
        poll();
    });
    d.method("toggleSelect", |this, args| {
        let id = js::arg_str(&args, 0);
        set_selected(&this, &id, !is_selected(&this, &id));
        js::set_str(&this, "focusId", &id);
        js::set_str(&this, "anchorId", &id);
        JsValue::UNDEFINED
    });
    // Shift-click on a checkbox: select the inclusive range of visible rows
    // from the anchor (the last plain click) through this one. Rows already
    // selected outside the range stay selected.
    d.method("selectRange", |this, args| {
        let id = js::arg_str(&args, 0);
        let anchor = js::get_str(&this, "anchorId");
        let ids = visible_ids(&this);
        let end = ids.iter().position(|i| i == &id);
        let start = if anchor.is_empty() {
            None
        } else {
            ids.iter().position(|i| i == &anchor)
        };
        if let (Some(a), Some(b)) = (start, end) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            select_ids(&this, &ids[lo..=hi]);
        } else {
            set_selected(&this, &id, true);
            js::set_str(&this, "anchorId", &id);
        }
        js::set_str(&this, "focusId", &id);
        JsValue::UNDEFINED
    });
    // A plain click on a row — its checkbox or its sender / subject / summary.
    // An unselected email becomes the whole selection; an already-selected one
    // is removed. Either way the row becomes the shift-click anchor.
    d.method("selectOnly", |this, args| {
        let id = js::arg_str(&args, 0);
        if id.is_empty() {
            return JsValue::UNDEFINED;
        }
        if is_selected(&this, &id) {
            set_selected(&this, &id, false);
        } else {
            let arr = Array::new();
            arr.push(&JsValue::from_str(&id));
            js::set(&this, "selected", &arr.into());
            mjs::redraw();
        }
        js::set_str(&this, "anchorId", &id);
        js::set_str(&this, "focusId", &id);
        JsValue::UNDEFINED
    });
    d.action("selectAll", |this, _args| {
        let arr = Array::new();
        for id in visible_ids(&this) {
            arr.push(&JsValue::from_str(&id));
        }
        js::set(&this, "selected", &arr.into());
        mjs::redraw();
    });
    // The toolbar's one button for both directions: empty selection → all,
    // any selection → none.
    d.action("toggleSelectAll", |this, _args| {
        let arr = Array::new();
        if !js::get_bool(&this, "allSelected") {
            for id in visible_ids(&this) {
                arr.push(&JsValue::from_str(&id));
            }
        }
        js::set(&this, "selected", &arr.into());
        mjs::redraw();
    });
    d.action("clearSelection", |this, _args| {
        js::set(&this, "selected", &Array::new().into());
    });
    d.action("refresh", |_this, _args| {
        bump_search();
        poll();
    });
    d.action("pull", |_this, _args| pull());
    d.action("push", |_this, _args| push());
    d.action("cancelQueue", |_this, args| {
        let id = js::arg_str(&args, 0);
        cancel(&id);
    });
    d.action("cancelAll", |_this, _args| cancel(""));
    // One row, one action: the row's own spam button. (`toggleRow` above is the
    // same idea for the two labels that have an on/off state.)
    d.method("queueRow", |this, args| {
        let id = js::arg_str(&args, 0);
        let op = js::arg_str(&args, 1);
        if !id.is_empty() && !op.is_empty() {
            queue_ids(&this, &[id], &op, "", "", "");
        }
        JsValue::UNDEFINED
    });
    // Run the disk rules over the target set and ask the model what each email
    // deserves. The rules decide *which emails get asked* and *what the question
    // says*; the answer is a recommendation the row shows, never an action that
    // happens by itself.
    d.action("evaluateRules", |this, _args| {
        let ids = target_ids(&this);
        if ids.is_empty() {
            session::toast("select emails first", "warn");
            return;
        }
        evaluate_rules(&ids);
    });
    // The row's own Rules button: one email, not the selection.
    d.method("evaluateRulesRow", |_this, args| {
        let id = js::arg_str(&args, 0);
        if id.is_empty() {
            return JsValue::UNDEFINED;
        }
        evaluate_rules(&[id]);
        JsValue::UNDEFINED
    });
    // The row's recommendation button: queue exactly what the model recommended.
    d.method("applyRec", |this, args| {
        let id = js::arg_str(&args, 0);
        apply_rec(&this, &id);
        JsValue::UNDEFINED
    });
    // `z`: press the recommendation button for the email on screen — the open
    // email while previewing, else the row `j`/`k` put the focus ring on.
    d.action("applyFocusedRec", |this, _args| {
        let open = js::get_str(&this, "openId");
        let id = if !open.is_empty() {
            open
        } else {
            js::get_str(&this, "focusId")
        };
        if id.is_empty() {
            session::toast("no focused email", "warn");
            return;
        }
        if !apply_rec(&this, &id) {
            session::toast("no recommendation on this email — press r first", "info");
        }
    });
    // `q`: flip read/unread on the target set, per email. Unread ones are
    // marked read and read ones unread, so a mixed selection still does the
    // right thing for each row.
    d.action("toggleRead", |this, _args| {
        toggle_read(&this);
    });
/// Queue exactly what the model recommended for one email. Shared by the
/// row's recommendation button and the `z` hotkey. Returns false when there
/// is no actionable recommendation (none yet, or a `skip` label).
fn apply_rec(mail: &JsValue, id: &str) -> bool {
    let rec = js::get(&js::get(mail, "recs"), id);
    if js::is_missing(&rec) {
        return false;
    }
    let op = js::get_str(&rec, "action");
    if op.is_empty() {
        return false;
    }
    if op == "skip" {
        session::toast("skip means leave it to you — nothing queued", "info");
        return false;
    }
    let folder = js::get_str(&rec, "folder");
    // The instruction stays in the queue's own vocabulary (so the row chip
    // reads `move→Statements`, same as a hand-made Move), and the provenance
    // — which rules said so, with what confidence — goes in the rationale,
    // where the outbox can show it.
    let why = format!(
        "rule → {} (confidence {:.2})",
        js::get_str(&rec, "label"),
        js::get_num(&rec, "confidence")
    );
    queue_ids(mail, &[id.to_string()], &op, &folder, &op, &why);
    // The recommendation has been asked for. The row button becomes Undo
    // until that queued action is taken back, which returns the row to the
    // untouched Rules state.
    js::set_bool(&js::get(mail, "undoIds"), id, true);
    true
}

fn toggle_read(mail: &JsValue) {
    let ids = target_ids(mail);
    if ids.is_empty() {
        session::toast("select at least one email first", "warn");
        return;
    }
    let viewing = !js::get_str(mail, "openId").is_empty();
    let mut to_read: Vec<String> = Vec::new();
    let mut to_unread: Vec<String> = Vec::new();
    for id in &ids {
        if email_is_unread(mail, id) {
            to_read.push(id.clone());
        } else {
            to_unread.push(id.clone());
        }
    }
    if !to_read.is_empty() {
        queue_ids(mail, &to_read, "read", "", "", "");
    }
    if !to_unread.is_empty() {
        queue_ids(mail, &to_unread, "unread", "", "", "");
    }
    // Like every other toolbar action: the preview acted, so leave it and
    // show the list where the queued row sits.
    if viewing {
        close_mail(mail);
    }
}

/// The unread flag for one email: the open message when it is the one on
/// screen, the list row otherwise.
fn email_is_unread(mail: &JsValue, id: &str) -> bool {
    if js::get_str(mail, "openId").as_str() == id {
        let open = js::get(mail, "open");
        if !js::is_missing(&open) {
            return js::get_bool(&open, "unread");
        }
    }
    row_bool(mail, id, "unread")
}

    // Undo the recommendation's queued action and forget that rules were run.
    d.method("undoRow", |this, args| {
        let id = js::arg_str(&args, 0);
        if id.is_empty() {
            return JsValue::UNDEFINED;
        }
        forget_row_fsm(&this, &id);
        cancel(&id);
        JsValue::UNDEFINED
    });
    d.action("clearRecs", |this, _args| {
        js::set(&this, "recs", &Object::new().into());
        js::set(&this, "undoIds", &Object::new().into());
        js::set_num(&this, "recCount", 0.0);
    });
    // New Rule: open the dialog. No argument = the selection (toolbar).
    // One argument = that email only (the row's own New Rule button).
    d.action("openNewRule", |this, args| {
        let row = js::arg_str(&args, 0);
        if row.is_empty() {
            if target_ids(&this).is_empty() {
                session::toast("select emails first", "warn");
                return;
            }
            js::set_str(&this, "ruleRow", "");
        } else {
            js::set_str(&this, "ruleRow", &row);
        }
        js::set_bool(&this, "ruleOpen", true);
        js::set_str(&this, "rulePrompt", "");
        js::set_str(&this, "ruleErr", "");
        // The textarea is shown by this write; focus it once the frame has
        // applied x-show, so the user can type immediately.
        rustex1_kit::lifecycle::after(16.0, focus_rule_prompt);
    });
    d.action("closeNewRule", |this, _args| {
        js::set_bool(&this, "ruleOpen", false);
        js::set_str(&this, "rulePrompt", "");
        js::set_str(&this, "ruleRow", "");
        js::set_str(&this, "ruleErr", "");
    });
    d.action("submitNewRule", |this, args| {
        if js::get_bool(&this, "ruleBusy") {
            return;
        }
        let row = js::get_str(&this, "ruleRow");
        let ids = if !row.is_empty() {
            vec![row]
        } else {
            target_ids(&this)
        };
        if ids.is_empty() {
            session::toast("select emails first", "warn");
            return;
        }
        // The narration is passed in (read live from the input by the page).
        // Fall back to the store binding if the caller sent nothing.
        let mut prompt = js::arg_str(&args, 0);
        if prompt.trim().is_empty() {
            prompt = js::get_str(&this, "rulePrompt");
        }
        if prompt.trim().is_empty() {
            session::toast("describe what the rule should do", "warn");
            return;
        }
        create_rule(&ids, &prompt);
    });
    // Run the phraser rules over the target set and ask the text LLM to
    // reword each match. The rewordings persist in the `phrase` table, so the
    // rows the next poll returns already carry them — unlike recommendations,
    // phrases survive refresh and restart.
    d.action("phraseRows", |this, _args| {
        let ids = target_ids(&this);
        if ids.is_empty() {
            session::toast("select emails first", "warn");
            return;
        }
        phrase_ids(&ids);
    });
    // Phrases persist, so clearing is explicit: the rows go back to their own
    // words on the next poll.
    d.action("clearPhrases", |_this, _args| {
        api::send("/api/phrases", Some("DELETE"), None, |v| {
            let m = mjs::store_ref("mail");
            if js::is_missing(&m) {
                return;
            }
            poll();
            session::toast(
                &format!(
                    "dropped {} stored phrase(s) — rows show their own words again",
                    js::get_num(&v, "removed") as usize
                ),
                "warn",
            );
        });
    });
    d.method("openMail", |this, args| {
        let id = js::arg_str(&args, 0);
        js::set_str(&this, "openId", &id);
        push_mail_history(&id);
        open_message(&id);
        JsValue::UNDEFINED
    });
    d.action("loadBody", |this, _args| {
        let id = js::get_str(&this, "openId");
        if !id.is_empty() {
            open_message(&id);
        }
    });
    d.action("closeMail", |this, _args| {
        close_mail(&this);
    });
    d.method("setOpenMode", |this, args| {
        js::set_str(&this, "openMode", &js::arg_str(&args, 0));
        crate::keys::schedule_paint();
        JsValue::UNDEFINED
    });
    d.method("moveFocus", |this, args| {
        // The hotkey table passes the direction as a string ("1" / "-1"),
        // and `as_f64` on a string is always None — so the default won and
        // `k` moved down just like `j`. Parse the string instead.
        let dir = js::arg_str(&args, 0).parse::<f64>().unwrap_or(1.0);
        move_focus(&this, dir);
        JsValue::UNDEFINED
    });
    d.action("toggleFocus", |this, _args| {
        let id = js::get_str(&this, "focusId");
        if !id.is_empty() {
            set_selected(&this, &id, !is_selected(&this, &id));
        }
    });
    d.action("openFocused", |this, _args| {
        let id = js::get_str(&this, "focusId");
        if !id.is_empty() {
            js::set_str(&this, "openId", &id);
            push_mail_history(&id);
            open_message(&id);
        }
    });
    // `op` is the server's vocabulary: archive | spam | delete | purge | read |
    // unread | star | unstar | important | unimportant | move.
    d.method("act", |this, args| {
        let op = js::arg_str(&args, 0);
        act(&this, &op);
        JsValue::UNDEFINED
    });
    // Row-level star / important: the same queue, just for one row. The
    // template passes `(id, field)` only — **this** side decides on/off by
    // reading the row's effective labels, so the row cannot disagree with the
    // mutation it queues. (A template that had to pass the current state was
    // how this method used to queue an empty `modify` for every click.)
    d.method("toggleRow", |this, args| {
        let id = js::arg_str(&args, 0);
        let field = js::arg_str(&args, 1);
        let label = if field == "important" { "IMPORTANT" } else { "STARRED" };
        let op = match (field.as_str(), !row_has_label(&this, &id, label)) {
            ("important", true) => "important",
            ("important", false) => "unimportant",
            (_, true) => "star",
            (_, false) => "unstar",
        };
        queue_ids(&this, &[id], op, "", "", "");
        JsValue::UNDEFINED
    });

    mjs::store("mail", &d.finish());
}

// ---------------------------------------------------------------------------
// ui — the action bar
// ---------------------------------------------------------------------------

fn ui() {
    let mut d = Def::new().named("ui store");
    d.str("action", "")
        .bool("open", false)
        .str("rowTarget", "")
        .str("destination", "")
        .str("instruction", "")
        .str("rationale", "")
        .str("flash", "")
        .str("err", "")
        .bool("submitting", false);

    // Second argument: a **row's** action. The toolbar acts on the selection or
    // the open email; a row's own Move… acts on that one email, which is why the
    // row can offer the folder button without the user having to select anything.
    d.method("openAction", |this, args| {
        let action = js::arg_str(&args, 0);
        let row = js::arg_str(&args, 1);
        js::set_str(&this, "action", &action);
        js::set_bool(&this, "open", true);
        js::set_str(&this, "err", "");
        js::set_str(&this, "rowTarget", &row);
        if !row.is_empty() {
            js::set_str(&mjs::store_ref("mail"), "focusId", &row);
        }
        // Prefill the instruction so Enter is enough; the destination select
        // carries a Move instead.
        let preset = match action.as_str() {
            "archive" | "spam" | "delete" | "purge" | "read" | "unread" | "star"
            | "unstar" | "important" | "unimportant" => action.clone(),
            _ => String::new(),
        };
        js::set_str(&this, "instruction", &preset);
        JsValue::UNDEFINED
    });
    d.action("closeAction", |this, _args| {
        js::set_str(&this, "action", "");
        js::set_bool(&this, "open", false);
        js::set_str(&this, "instruction", "");
        js::set_str(&this, "rationale", "");
        js::set_str(&this, "err", "");
        js::set_str(&this, "rowTarget", "");
        // The destination used to survive closing: Move… opened a second time
        // still had last time's folder selected, so a submit went somewhere the
        // user had already stopped looking at.
        js::set_str(&this, "destination", "");
    });
    d.action("submit", |this, _args| {
        let action = js::get_str(&this, "action");
        if action.is_empty() {
            return;
        }
        let destination = js::get_str(&this, "destination");
        if action == "move" && destination.is_empty() {
            js::set_str(&this, "err", "choose a destination folder");
            return;
        }
        let instruction = js::get_str(&this, "instruction");
        let rationale = js::get_str(&this, "rationale");
        let mail = mjs::store_ref("mail");
        if js::is_missing(&mail) {
            return;
        }
        let ids = if !js::get_str(&this, "rowTarget").is_empty() {
            vec![js::get_str(&this, "rowTarget")]
        } else {
            target_ids(&mail)
        };
        if ids.is_empty() {
            js::set_str(&this, "err", "select at least one email first");
            return;
        }
        let viewing = !js::get_str(&mail, "openId").is_empty();
        queue_ids(&mail, &ids, &action, &destination, &instruction, &rationale);
        if viewing {
            close_mail(&mail);
        }
        js::set_str(&this, "rowTarget", "");
        js::set_str(&this, "flash", &format!("queued {} · press PUSH", ids.len()));
        flash_clear();
        // The bar has done its job. Left open after a row's Move… it would sit
        // there showing "no target", because the target it was aimed at is gone
        // the moment the action queues.
        js::set_str(&this, "action", "");
        js::set_bool(&this, "open", false);
        js::set_str(&this, "instruction", "");
        js::set_str(&this, "rationale", "");
        js::set_str(&this, "destination", "");
    });

    d.getter("targetSummary", |this, _| {
        let mail = mjs::store_ref("mail");
        let row = js::get_str(&this, "rowTarget");
        let text = if !row.is_empty() {
            match row_subject(&mail, &row) {
                Some(subject) => format!("1 row · \u{201c}{subject}\u{201d}"),
                None => "1 row".to_string(),
            }
        } else {
            let ids = target_ids(&mail);
            match ids.len() {
                0 => "no target — select emails first".to_string(),
                1 => {
                    let id = &ids[0];
                    if js::get_str(&mail, "focusId") == *id && selected_ids(&mail).is_empty() {
                        match row_subject(&mail, id) {
                            Some(subject) => format!("1 focused row · \u{201c}{subject}\u{201d}"),
                            None => "1 focused row".to_string(),
                        }
                    } else {
                        "1 open email".to_string()
                    }
                }
                n => format!("{n} selected"),
            }
        };
        JsValue::from_str(&text)
    });

    mjs::store("ui", &d.finish());
}

/// The server answers "does this row carry that label *right now*, pending
/// mutations included?" with three booleans — `unread`, `starred`, `important`
/// — computed from `effLabels`. Read those instead of digging through
/// `effLabels`, which is an array of **strings**: `arr[i].name` on a string is
/// undefined, so every label test quietly answered "no".
fn row_bool(mail: &JsValue, id: &str, field: &str) -> bool {
    let rows = match js::get(mail, "rows").dyn_ref::<Array>().cloned() {
        Some(r) => r,
        None => return false,
    };
    for i in 0..rows.length() {
        let row = rows.get(i);
        if js::get_str(&row, "id") == id {
            return js::get_bool(&row, field);
        }
    }
    false
}

/// Rows drawn at once. The toolbar pager steps through the filtered list
/// in chunks of this size.
const PAGE_SIZE: usize = 20;

fn filtered_rows(mail: &JsValue) -> Array {
    let out = Array::new();
    let Some(list) = js::get(mail, "rows").dyn_ref::<Array>().cloned() else {
        return out;
    };
    for i in 0..list.length() {
        let row = list.get(i);
        if matches_filter(mail, &row) {
            out.push(&row);
        }
    }
    out
}

fn page_count(total: usize) -> usize {
    if total == 0 {
        1
    } else {
        (total + PAGE_SIZE - 1) / PAGE_SIZE
    }
}

fn page_index(mail: &JsValue, total: usize) -> usize {
    let count = page_count(total);
    let page = js::get_num(mail, "page").max(0.0) as usize;
    page.min(count - 1)
}

fn page_rows(mail: &JsValue) -> Array {
    let all = filtered_rows(mail);
    let total = all.length() as usize;
    let start = page_index(mail, total) * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    let out = Array::new();
    for i in start..end {
        out.push(&all.get(i as u32));
    }
    out
}

/// Keep the user's place across a refresh. The focused row may be gone
/// (pushed out of the inbox); the ring then moves to the row now sitting
/// where it was, clamped to the end of the new list. The selection is pruned
/// to ids the refresh still carries, and the page is clamped to the last
/// page that still exists.
fn restore_place(mail: &JsValue, old_focus_id: &str, old_focus_idx: Option<usize>) {
    let list = filtered_rows(mail);
    let total = list.length() as usize;
    if total == 0 {
        js::set_str(mail, "focusId", "");
    } else {
        let mut still_there = false;
        for i in 0..list.length() {
            if js::get_str(&list.get(i), "id") == *old_focus_id {
                still_there = true;
                break;
            }
        }
        if !still_there {
            if let Some(i) = old_focus_idx {
                let at = i.min(total - 1);
                let id = js::get_str(&list.get(at as u32), "id");
                js::set_str(mail, "focusId", &id);
                if let Some(el) = js::query(&format!(".mail-row[data-id=\"{id}\"]")) {
                    let opts = js::new_object();
                    js::set_str(&opts, "behavior", "smooth");
                    js::call(&el, "scrollIntoView", &[opts.into()]);
                }
            }
        }
    }
    // Prune the selection to ids the new rows still carry.
    let rows = js::get(mail, "rows");
    if let Some(all) = rows.dyn_ref::<Array>() {
        let sel = js::get(mail, "selected");
        if let Some(arr) = sel.dyn_ref::<Array>() {
            let keep = Array::new();
            for i in 0..arr.length() {
                let id = arr.get(i).as_string().unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                for j in 0..all.length() {
                    if js::get_str(&all.get(j), "id") == id {
                        keep.push(&JsValue::from_str(&id));
                        break;
                    }
                }
            }
            js::set(mail, "selected", &keep.into());
        }
    }
    // Clamp the page: the refresh may have shrunk the list.
    let count = page_count(total);
    let page = js::get_num(mail, "page").max(0.0) as usize;
    if page >= count {
        js::set_num(mail, "page", (count - 1) as f64);
    }
}

fn matches_filter(this: &JsValue, row: &JsValue) -> bool {
    // filter
    let filter = js::get_str(this, "filter");
    if !filter.is_empty() {
        match filter.as_str() {
            "unread" => {
                if !js::get_bool(row, "unread") {
                    return false;
                }
            }
            "starred" => {
                if !js::get_bool(row, "starred") {
                    return false;
                }
            }
            "pending" => {
                if js::get_num(row, "pendingCount") as i64 <= 0 {
                    return false;
                }
            }
            "inbox" => {
                // Queueing a move/archive removes INBOX from `effLabels` so the
                // row can show what Push will do. It must not leave the list
                // until Push: the stored `inInbox` flag is the mirror's word,
                // and the row button turns into Undo.
                if !js::get_bool(row, "inInbox") {
                    return false;
                }
            }
            _ => {}
        }
    }
    // search q
    let q = js::get_str(this, "q").trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {}",
        js::get_str(row, "fromName"),
        js::get_str(row, "fromEmail"),
        js::get_str(row, "subject"),
        js::get_str(row, "snippet")
    )
    .to_lowercase();
    hay.contains(&q)
}

/// `GET /api/mail` → the `mail` store.
pub fn poll() {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) {
        return;
    }
    js::set_bool(&m, "loading", true);
    let filter = js::get_str(&m, "filter");
    let q = js::get_str(&m, "q");
    // Remember what is being fetched: the debounced search uses this to skip a
    // poll the 4 s tick has already done for the same filter + q.
    let key = format!("{filter}\u{1}{q}");
    LAST_FETCH.with(|k| *k.borrow_mut() = key.clone());
    let path = format!(
        "/api/mail?filter={}&q={}&limit=1000",
        urlencode(&filter),
        urlencode(&q)
    );
    api::get(&path, None, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        // Two polls can be in flight at once — a filter click and a search, say
        // — and they may answer out of order. A response for a query the list no
        // longer shows is dropped, or the older answer repaints the list with
        // another query's rows (and its `matched` with another query's total).
        if fetch_key(&m) != key {
            return;
        }
        js::set_bool(&m, "loading", false);
        if !js::get_bool(&v, "ok") {
            js::set_str(&m, "error", &js::get_str(&v, "error"));
            return;
        }
        js::set_str(&m, "error", "");
        // Where the focus ring sat, before the new rows replace the old ones.
        // Pushed rows leave the list, so without this the focus points at a
        // row that is gone and `j`/`k` restart from an edge.
        let old_focus_id = js::get_str(&m, "focusId");
        let old_focus_idx = {
            let list = filtered_rows(&m);
            let mut at = None;
            for i in 0..list.length() {
                if js::get_str(&list.get(i), "id") == old_focus_id {
                    at = Some(i as usize);
                    break;
                }
            }
            at
        };
        js::set(&m, "rows", &js::get(&v, "rows"));
        restore_place(&m, &old_focus_id, old_focus_idx);
        js::set(&m, "counts", &js::get(&v, "counts"));
        js::set_num(&m, "matched", js::get_num(&v, "matched"));
        js::set(&m, "lastPull", &js::get(&v, "lastPull"));
        js::set(&m, "lastPush", &js::get(&v, "lastPush"));
        js::set_num(&m, "at", js::get_num(&v, "at"));
    });
}
/// `GET /api/rules` → the Rules button badge: how many rules parsed.
pub fn poll_rules() {
    api::get("/api/rules", None, |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) || !js::get_bool(&v, "ok") {
            return;
        }
        if let Some(list) = js::get(&v, "rules").dyn_ref::<Array>() {
            js::set_num(&m, "ruleCount", list.length() as f64);
        }
    });
}
/// `GET /api/labels` → the Move dropdown.
pub fn poll_labels() {
    api::get("/api/labels", None, |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) || !js::get_bool(&v, "ok") {
            return;
        }
        js::set(&m, "labels", &js::get(&v, "labels"));
    });
}

/// Drop a row's rule-run and its Undo flag, so the button is the Rules
/// button again — the state before anything was asked about this email.
fn focus_rule_prompt() {
    if let Some(el) = js::query("input.rule-prompt") {
        js::call(&el, "focus", &[]);
    }
}

fn drop_key(obj: &JsValue, id: &str) {
    if let Some(o) = obj.dyn_ref::<Object>() {
        let _ = js_sys::Reflect::delete_property(o, &JsValue::from_str(id));
    }
}

fn forget_row_fsm(mail: &JsValue, id: &str) {
    drop_key(&js::get(mail, "recs"), id);
    drop_key(&js::get(mail, "undoIds"), id);
}

/// `POST /api/rules/eval` → `mail.recs`, one recommendation per message id.
///
/// The server re-reads `rules/*.yaml` on every call, so editing a rule's wording
/// takes a save, not a restart. Recommendations land in a map keyed by id; the
/// row renders its entry, and nothing is queued until the row's button is
/// pressed.
pub fn evaluate_rules(ids: &[String]) {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) || ids.is_empty() {
        return;
    }
    js::set_bool(&m, "recBusy", true);
    // A fresh run replaces whatever phase the row was in.
    for id in ids {
        drop_key(&js::get(&m, "undoIds"), id);
    }
    let body = js::new_object();
    let arr = Array::new();
    for id in ids {
        arr.push(&JsValue::from_str(id));
    }
    js::set(&body, "ids", &arr.into());
    let asked = ids.len();
    api::post_json("/api/rules/eval", &body, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "recBusy", false);
        if !js::get_bool(&v, "ok") {
            // A model that is not running is a fact to report, not a silent gap:
            // the rows would otherwise just have no recommendation button.
            session::toast(
                &format!("rules: {}", js::get_str(&v, "error")),
                "err",
            );
            return;
        }
        let recs = js::get(&m, "recs");
        let results = js::get(&v, "results");
        let mut answered = 0usize;
        let mut errored = 0usize;
        if let Some(list) = results.dyn_ref::<Array>() {
            for i in 0..list.length() {
                let r = list.get(i);
                let id = js::get_str(&r, "id");
                if id.is_empty() {
                    continue;
                }
                js::set(&recs, &id, &r);
                if js::get_bool(&r, "ok") && !js::get_str(&r, "action").is_empty() {
                    answered += 1;
                } else {
                    errored += 1;
                }
            }
        }
        if let Some(o) = recs.dyn_ref::<Object>() {
            // Only recommendations you can act on count: a row no rule matched
            // has an entry (so the detail view can say so) but no button.
            // recCount is kept for the clear-button visibility; the Rules
            // button badge shows ruleCount instead.
            let values = Object::values(o);
            let mut actionable = 0usize;
            for i in 0..values.length() {
                if !js::get_str(&values.get(i), "action").is_empty() {
                    actionable += 1;
                }
            }
            js::set_num(&m, "recCount", actionable as f64);
        }
        // The server re-reads rules/*.yaml on every eval, so refresh the
        // rule count here too — editing rules while the app runs updates it.
        if let Some(list) = js::get(&v, "rules").dyn_ref::<Array>() {
            js::set_num(&m, "ruleCount", list.length() as f64);
        }
        let note = if errored > 0 {
            format!("{answered} recommended, {errored} error(s) — see the row")
        } else {
            format!("{answered} of {asked} recommended")
        };
        session::toast(&format!("rules: {note}"), "info");
        mjs::redraw();
    });
}

/// `POST /api/rules/new` → the text LLM drafts a rule from the narration and the
/// example emails, and the server writes it to `rules/*.yaml`. On success the
/// dialog closes and the rule badge refreshes.
pub fn create_rule(ids: &[String], prompt: &str) {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) || ids.is_empty() {
        return;
    }
    js::set_bool(&m, "ruleBusy", true);
    let body = js::new_object();
    let arr = Array::new();
    for id in ids {
        arr.push(&JsValue::from_str(id));
    }
    js::set(&body, "ids", &arr.into());
    js::set_str(&body, "prompt", prompt);
    let asked_ids = ids.to_vec();
    api::post_json("/api/rules/new", &body, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "ruleBusy", false);
        if !js::get_bool(&v, "ok") {
            let err = js::get_str(&v, "error");
            let err = if err.is_empty() {
                "the rule was not created".to_string()
            } else {
                err
            };
            // Leave the dialog open and say why, so a failed submit does not
            // look like the button merely blinked.
            js::set_str(&m, "ruleErr", &err);
            session::toast(&err, "err");
            mjs::redraw();
            return;
        }
        let file = js::get_str(&v, "file");
        // The rows this rule was written for go back to the Rules button, so
        // the new rule can be run against them.
        for id in &asked_ids {
            forget_row_fsm(&m, id);
        }
        js::set_bool(&m, "ruleOpen", false);
        js::set_str(&m, "rulePrompt", "");
        js::set_str(&m, "ruleRow", "");
        js::set_str(&m, "ruleErr", "");
        session::toast(&format!("wrote rule {file}"), "ok");
        poll_rules();
        // Convenience: run Rules over the emails the rule was written for, so
        // the fresh recommendation lands on the rows without another click.
        evaluate_rules(&asked_ids);
        mjs::redraw();
    });
}

/// `POST /api/phrasers/eval` → phrases land in the `phrase` table, so a plain
/// `poll()` brings them onto the rows — no separate map to keep in sync.
pub fn phrase_ids(ids: &[String]) {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) || ids.is_empty() {
        return;
    }
    js::set_bool(&m, "phraseBusy", true);
    let body = js::new_object();
    let arr = Array::new();
    for id in ids {
        arr.push(&JsValue::from_str(id));
    }
    js::set(&body, "ids", &arr.into());
    let asked = ids.len();
    api::post_json("/api/phrasers/eval", &body, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "phraseBusy", false);
        if !js::get_bool(&v, "ok") {
            // The usual reason is the workflow itself: the text model is not
            // running because the System One model is. Say so plainly.
            let first = js::get(&v, "results")
                .dyn_ref::<Array>()
                .map(|a| js::get_str(&a.get(0), "error"))
                .unwrap_or_default();
            let why = if first.is_empty() {
                js::get_str(&v, "error")
            } else {
                first
            };
            session::toast(&format!("phrase: {why}"), "err");
            return;
        }
        let n = js::get_num(&v, "phrased") as usize;
        poll();
        session::toast(&format!("phrased {n} of {asked} — rows show the new words"), "ok");
        mjs::redraw();
    });
}

/// One history entry per open email, same URL. Browser Back / the mouse back
/// button then pop that entry and land on the list — not on a second, older page.
fn push_mail_history(id: &str) {
    if id.is_empty() || history_mail_id() == id {
        return;
    }
    let state = js::new_object();
    js::set_str(&state, "mail1", id);
    let href = js::get_str(&js::get(&js::global(), "location"), "href");
    let hist = js::get(&js::global(), "history");
    let _ = js::call(
        &hist,
        "pushState",
        &[state.into(), JsValue::from_str(""), JsValue::from_str(&href)],
    );
}

fn history_mail_id() -> String {
    let hist = js::get(&js::global(), "history");
    let state = js::get(&hist, "state");
    if js::is_missing(&state) || state.is_null() {
        return String::new();
    }
    js::get_str(&state, "mail1")
}

fn close_mail(mail: &JsValue) {
    let open = js::get_str(mail, "openId");
    if open.is_empty() {
        return;
    }
    js::set_str(mail, "openId", "");
    js::set(mail, "open", &Object::new().into());
    // Pop only the entry this open pushed. A second `history.back()` is what
    // made one Back click leave the app as well as the message.
    if history_mail_id() == open {
        let hist = js::get(&js::global(), "history");
        let _ = js::call(&hist, "back", &[]);
    }
}

/// `popstate` already moved history. Clear the detail view and do not go back
/// again.
pub fn dismiss_open() {
    let mail = mjs::store_ref("mail");
    if js::is_missing(&mail) {
        return;
    }
    let open = js::get_str(&mail, "openId");
    if open.is_empty() {
        return;
    }
    // If this pop landed on the entry for the email still open, it was not our
    // close. Leave it.
    if history_mail_id() == open {
        return;
    }
    js::set_str(&mail, "openId", "");
    js::set(&mail, "open", &Object::new().into());
}

/// `GET /api/mail-one?id=…&full=1` → the detail view. `full` asks the server to
/// fetch the body from Gmail **once** and cache it in SQLite, so the second and
/// every later open of this email is a pure local read.
pub fn open_message(id: &str) {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) {
        return;
    }
    js::set_bool(&m, "openLoading", true);
    js::set(&m, "open", &Object::new().into());
    let path = format!("/api/mail-one?id={}&full=1", urlencode(id));
    api::get(&path, None, |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "openLoading", false);
        js::set(&m, "open", &js::get(&v, "message"));
        js::set_str(
            &m,
            "openMode",
            if js::get_str(&js::get(&v, "message"), "bodyKind") == "html" {
                "html"
            } else {
                "text"
            },
        );
        crate::keys::schedule_paint();
        if !js::get_bool(&v, "ok") {
            session::toast(&js::get_str(&v, "error"), "err");
        }
    });
}

// ---------------------------------------------------------------------------
// writes
// ---------------------------------------------------------------------------

fn queue_ids(
    _mail: &JsValue,
    ids: &[String],
    op: &str,
    destination: &str,
    instruction: &str,
    rationale: &str,
) {
    let op = op.to_string();
    let body = js::new_object();
    let arr = Array::new();
    for id in ids {
        arr.push(&JsValue::from_str(id));
    }
    js::set(&body, "ids", &arr.into());
    js::set_str(&body, "op", &op);
    js::set_str(&body, "destination", destination);
    js::set_str(&body, "instruction", instruction);
    js::set_str(&body, "rationale", rationale);

    api::post_json("/api/queue", &body, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        if !js::get_bool(&v, "ok") {
            let e = js::get_str(&v, "error");
            session::toast(&format!("could not queue {op}: {e}"), "err");
            return;
        }
        // The response carries the affected rows already carrying their pending
        // state — splice them in and the screen is right before Gmail is.
        let updated = js::get(&v, "rows");
        merge_rows(&m, &updated);
        // The open email reads `mail.open`, not the list row. Copy the queued
        // row onto it so the detail view shows "queued: delete" immediately.
        let open_id = js::get_str(&m, "openId");
        if !open_id.is_empty() {
            if let Some(arr) = updated.dyn_ref::<Array>() {
                for i in 0..arr.length() {
                    let row = arr.get(i);
                    if js::get_str(&row, "id") == open_id {
                        js::set(&m, "open", &row);
                        break;
                    }
                }
            }
        }
        js::set(&m, "counts", &js::get(&v, "counts"));
        let n = js::get_num(&v, "queued") as usize;
        session::toast(&format!("queued {n} × {op} — press PUSH to send"), "info");
        mjs::redraw();
    });
}

/// Replace the rows the server just changed, keeping the array's order.
fn merge_rows(mail: &JsValue, updated: &JsValue) {
    let Some(rows) = js::get(mail, "rows").dyn_ref::<Array>().cloned() else {
        return;
    };
    let Some(fresh) = updated.dyn_ref::<Array>() else {
        return;
    };
    if fresh.length() == 0 {
        return;
    }
    let out = Array::new();
    for i in 0..rows.length() {
        let row = rows.get(i);
        let id = js::get_str(&row, "id");
        let mut replacement = None;
        for j in 0..fresh.length() {
            let cand = fresh.get(j);
            if js::get_str(&cand, "id") == id {
                replacement = Some(cand);
                break;
            }
        }
        out.push(&replacement.unwrap_or(row));
    }
    js::set(mail, "rows", &out.into());
}

fn cancel(message_id: &str) {
    let path = if message_id.is_empty() {
        "/api/queue?all=1".to_string()
    } else {
        format!("/api/queue?message_id={}", urlencode(message_id))
    };
    let for_me = message_id.to_string();
    api::send(&path, Some("DELETE"), None, move |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set(&m, "counts", &js::get(&v, "counts"));
        // `poll()` refreshes `rows`, not `open`: cancelling from the detail view
        // otherwise left its "queued:" list showing an action the outbox no
        // longer holds. Re-read the email that is on screen.
        let open = js::get_str(&m, "openId");
        if !open.is_empty() && (for_me.is_empty() || for_me == open) {
            open_message(&open);
        }
        poll();
        session::toast(
            &format!(
                "removed {} queued action(s)",
                js::get_num(&v, "removed") as usize
            ),
            "warn",
        );
    });
}

// ---------------------------------------------------------------------------
// pull · push
// ---------------------------------------------------------------------------

pub fn pull() {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) {
        return;
    }
    if js::get_bool(&m, "pulling") {
        return;
    }
    js::set_bool(&m, "pulling", true);
    let body = js::new_object();
    js::set_str(&body, "query", &js::get_str(&m, "query"));
    js::set_bool(&body, "full", js::get_bool(&m, "fullBodies"));
    js::set_num(&body, "max", js::get_num(&m, "max"));
    js::set_num(&body, "rate", js::get_num(&m, "rate"));
    api::post_json("/api/pull", &body, |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "pulling", false);
        js::set(&m, "lastPull", &v);
        let ms = js::get_num(&v, "totalMs") as i64;
        let listed = js::get_num(&v, "listed") as usize;
        let fetched = js::get_num(&v, "fetched") as usize;
        let backlog = js::get_num(&v, "backlog") as usize;
        if js::get_bool(&v, "ok") {
            let mut msg = format!(
                "pulled {listed} ids · {fetched} fetched in {}s",
                (ms as f64 / 1000.0).round()
            );
            if backlog > 0 {
                msg.push_str(&format!(" · backfill {backlog} to go"));
            }
            session::toast(&msg, "ok");
        } else {
            let errs = js::get(&v, "errors");
            let first = errs
                .dyn_ref::<Array>()
                .map(|a| a.get(0))
                .unwrap_or(JsValue::UNDEFINED);
            session::toast(
                &format!("pull stopped: {}", js::describe(&first)),
                "err",
            );
        }
        poll();
    });
}

pub fn push() {
    let m = mjs::store_ref("mail");
    if js::is_missing(&m) {
        return;
    }
    if js::get_bool(&m, "pushing") {
        return;
    }
    js::set_bool(&m, "pushing", true);
    let body = js::new_object();
    js::set_bool(&body, "retryFailed", true);
    api::post_json("/api/push", &body, |v| {
        let m = mjs::store_ref("mail");
        if js::is_missing(&m) {
            return;
        }
        js::set_bool(&m, "pushing", false);
        js::set(&m, "lastPush", &v);
        let applied = js::get_num(&v, "applied") as usize;
        let failed = js::get_num(&v, "failed") as usize;
        let batches = js::get_num(&v, "batches") as usize;
        if js::get_bool(&v, "ok") {
            session::toast(
                &format!(
                    "pushed {applied} action(s) in {} batches",
                    batches
                ),
                "ok",
            );
        } else {
            session::toast(&format!("push: {applied} applied, {failed} failed"), "err");
        }
        poll();
    });
}

// ---------------------------------------------------------------------------
// selection / focus helpers
// ---------------------------------------------------------------------------

fn is_selected(mail: &JsValue, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let sel = js::get(mail, "selected");
    if let Some(arr) = sel.dyn_ref::<Array>() {
        (0..arr.length()).any(|i| {
            let s = arr.get(i).as_string().unwrap_or_default();
            s == id
        })
    } else {
        false
    }
}


fn set_selected(mail: &JsValue, id: &str, on: bool) {
    let mut next: Vec<String> = selected_ids(mail);
    if on {
        if !next.iter().any(|i| i == id) {
            next.push(id.to_string());
        }
    } else {
        next.retain(|i| i != id);
    }
    let arr = Array::new();
    for id in next {
        arr.push(&JsValue::from_str(&id));
    }
    js::set(mail, "selected", &arr.into());
    mjs::redraw();
}

/// Add every id in the range. One write, so a long shift-click does not redraw
/// once per row.
fn select_ids(mail: &JsValue, ids: &[String]) {
    let mut next: Vec<String> = selected_ids(mail);
    for id in ids {
        if !next.iter().any(|i| i == id) {
            next.push(id.clone());
        }
    }
    let arr = Array::new();
    for id in next {
        arr.push(&JsValue::from_str(&id));
    }
    js::set(mail, "selected", &arr.into());
    mjs::redraw();
}
/// Every id the list actually **shows** — the set select-all covers. Reading
/// `rows` instead selected rows the current filter had hidden: "select all"
/// with the starred chip on queued actions against emails off screen.
fn visible_ids(mail: &JsValue) -> Vec<String> {
    let rows = page_rows(mail);
    let mut out = Vec::new();
    for i in 0..rows.length() {
        let id = js::get_str(&rows.get(i), "id");
        if !id.is_empty() {
            out.push(id);
        }
    }
    out
}

/// The subject of one row, for the action bar's "targets:" line: a Move… opened
/// from a row should say *which* row it will move.
fn row_subject(mail: &JsValue, id: &str) -> Option<String> {
    let rows = js::get(mail, "rows").dyn_ref::<Array>()?.clone();
    for i in 0..rows.length() {
        let row = rows.get(i);
        if js::get_str(&row, "id") == id {
            let subject = js::get_str(&row, "subject");
            let subject = if subject.is_empty() { "(no subject)" } else { &subject };
            let cut: String = subject.chars().take(60).collect();
            return Some(if cut.chars().count() < subject.chars().count() {
                format!("{cut}…")
            } else {
                cut
            });
        }
    }
    None
}

fn selected_ids(mail: &JsValue) -> Vec<String> {
    let sel = js::get(mail, "selected");
    if let Some(arr) = sel.dyn_ref::<Array>() {
        (0..arr.length()).map(|i| arr.get(i).as_string().unwrap_or_default()).collect()
    } else {
        Vec::new()
    }
}

fn move_focus(mail: &JsValue, dir: f64) {
    let list = page_rows(mail);
    if list.length() == 0 {
        return;
    }
    let focus = js::get_str(mail, "focusId");
    let mut idx: i64 = -1;
    for i in 0..list.length() {
        if js::get_str(&list.get(i), "id") == focus {
            idx = i as i64;
            break;
        }
    }
    let n = list.length() as i64;
    if n == 0 {
        return;
    }
    idx = if idx < 0 {
        if dir > 0.0 {
            0
        } else {
            n - 1
        }
    } else {
        (idx + dir as i64).rem_euclid(n)
    };
    let id = js::get_str(&list.get(idx as u32), "id");
    js::set_str(mail, "focusId", &id);
    // Keep the focused row on screen. The class is written by the template, so
    // this is a read of what m-js already rendered, not a second source of truth.
    if let Some(el) = js::query(&format!(".mail-row[data-id=\"{id}\"]")) {
        let opts = js::new_object();
        js::set_str(&opts, "behavior", "smooth");
        js::call(&el, "scrollIntoView", &[opts.into()]);
    }
}

/// One toolbar click or hotkey: resolve the target set and queue the action.
fn act(mail: &JsValue, op: &str) {
    let ids = target_ids(mail);
    if ids.is_empty() {
        session::toast("select at least one email first", "warn");
        return;
    }
    let viewing = !js::get_str(mail, "openId").is_empty();
    queue_ids(mail, &ids, op, "", "", "");
    // The toolbar acted on the email that was on screen. Leave the preview;
    // the list is where the queued row (and Undo) can be seen.
    if viewing {
        close_mail(mail);
    }
}

/// What an action acts on, in the order the reader means it: the email that is
/// open on screen, or the selection, or — for the keyboard, where `j`/`k` put a
/// ring on one row — the focused row. An open email always wins: the list is
/// hidden while the detail view is up, so any leftover selection is invisible —
/// clicking Archive must archive the email the user is looking at, not rows
/// they cannot see. Without the focused-row fallback `j j a` archived nothing
/// and merely toasted, even though the UI was pointing at a row.
fn target_ids(mail: &JsValue) -> Vec<String> {
    let open = js::get_str(mail, "openId");
    if !open.is_empty() {
        return vec![open];
    }
    let ids = selected_ids(mail);
    if !ids.is_empty() {
        return ids;
    }
    let focus = js::get_str(mail, "focusId");
    if !focus.is_empty() {
        return vec![focus];
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// small utilities
// ---------------------------------------------------------------------------

/// Does the cached row currently carry this label, **pending mutations
/// included**? `effLabels` is the server's answer to exactly that question.
fn row_has_label(mail: &JsValue, id: &str, label: &str) -> bool {
    let field = match label {
        "STARRED" => "starred",
        "IMPORTANT" => "important",
        "UNREAD" => "unread",
        _ => "",
    };
    if field.is_empty() {
        return false;
    }
    row_bool(mail, id, field)
}

/// "just now" / "4m ago" / "3h ago" / "2d ago" out of a store object carrying an
/// `at` epoch-ms field.
fn ago(store: &JsValue) -> JsValue {
    let at = js::get_num(store, "at");
    if at <= 0.0 {
        return JsValue::from_str("—");
    }
    let secs = ((Date::now() - at) / 1000.0).max(0.0);
    let text = if secs < 45.0 {
        "just now".to_string()
    } else if secs < 3600.0 {
        format!("{}m ago", (secs / 60.0).round() as i64)
    } else if secs < 86400.0 {
        format!("{}h ago", (secs / 3600.0).round() as i64)
    } else {
        format!("{}d ago", (secs / 86400.0).round() as i64)
    };
    JsValue::from_str(&text)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

thread_local! {
    /// Bumped by anything that changes *what the list should show* (a keystroke,
    /// a filter chip, refresh). A pending debounce compares its own generation
    /// against this and stands down if a newer request won.
    static SEARCH_GEN: Cell<u64> = const { Cell::new(0) };
    /// The `filter + q` the last `poll()` asked for.
    static LAST_FETCH: RefCell<String> = const { RefCell::new(String::new()) };
}

fn bump_search() -> u64 {
    SEARCH_GEN.with(|g| {
        let n = g.get() + 1;
        g.set(n);
        n
    })
}

fn fetch_key(mail: &JsValue) -> String {
    format!(
        "{}\u{1}{}",
        js::get_str(mail, "filter"),
        js::get_str(mail, "q")
    )
}

thread_local! {
    /// Which `submit()` a flash line belongs to. The old `FLASH_SEEN` boolean
    /// let the first submit arm the timer and blocked every later one, so a
    /// second submit inside 2.6 s wrote a flash nothing would ever clear.
    static FLASH_GEN: Cell<u64> = const { Cell::new(0) };
}

fn bump_flash() -> u64 {
    FLASH_GEN.with(|g| {
        let n = g.get() + 1;
        g.set(n);
        n
    })
}

/// The action bar's confirmation line is transient; the store owns the text and
/// this timer clears it — but only if no newer submit has taken the line over.
fn flash_clear() {
    let gen = bump_flash();
    rustex1_kit::lifecycle::after(2600.0, move || {
        if FLASH_GEN.with(|g| g.get()) != gen {
            return;
        }
        let u = mjs::store_ref("ui");
        if !js::is_missing(&u) {
            js::set_str(&u, "flash", "");
        }
    });
}
