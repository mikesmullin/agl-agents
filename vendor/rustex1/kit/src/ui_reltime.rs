//! `ui-reltime` — a relative timestamp that rewrites itself every second.
//!
//! Ported from Mike's canonical `tmp/psicom-c-ui/ui-reltime.mjs` (the component
//! AJCS and AVCS use), reduced to the half that is *not* already in this kit.
//! The original owns its hover bubble too; here that belongs to
//! [`crate::ui_tooltip`], which already measures the anchor, flips on short
//! headroom and clamps to the viewport. Two widgets implementing placement is how
//! the two drift apart, so composition happens wherever the cells are built:
//!
//! ```text
//! Node::host("ui-tooltip").bind("label", "cell.abs")   // long format, static
//!     .child(Node::host("ui-reltime").bind("at", "cell.at"))  // "3hr ago", ticking
//! ```
//!
//! The split is not arbitrary: the absolute time is a fixed instant and never
//! changes, so it is a plain prop written once by the host. Only the relative
//! label needs a clock.
//!
//! ## The m-js fast-update contract
//!
//! Same rule the tooltip follows — m-js renders the shell once and the tick is
//! the **sole writer** of the label's text. There is deliberately no `x-text` on
//! the node: a binding there would fight the tick, and whoever lost would leave a
//! stale "0sec ago" on screen. The tick re-reads `at` each beat, so a host that
//! changes the prop is reflected within a second with no re-render.
//!
//! The scope cannot reach its own element, so the instance tags its node with a
//! unique `data-ui-reltime` minted in the **factory** (one per instance).
//!
//! But there is exactly **one** timer per page, not one per instance: `init()`
//! subscribes to a shared tick (`ensure_tick`) that takes a single
//! `querySelectorAll` and a single `Date.now()` and updates every instance in one
//! step. See [`ensure_tick`] for why that is a correctness fix as well as a cost
//! one. `ui_tooltip` still creates a chain per instance — the same shape, waiting
//! for the same treatment.

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;

use crate::h::m;
use crate::js;
use crate::lifecycle;
use crate::mjs::{self, Def};

pub const NAME: &str = "ui-reltime";

/// 1 Hz — the smallest interval at which any bucket below can change.
const TICK_MS: f64 = 1000.0;

thread_local! {
    /// tag → component scope. The shared tick walks the **DOM** and looks scopes
    /// up by tag, so this map is never iterated to find work — the document is.
    static SCOPES: RefCell<HashMap<String, JsValue>> = RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// the boot whose shared tick is running. Keyed on the token rather than a
    /// bool so a hot swap starts a fresh chain instead of concluding one is
    /// already going — the old chain stops itself via `am_current_token`, and a
    /// bool would leave the new boot with no timer at all.
    static TICK_FOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The attribute each instance's root node is tagged with, and the only thing the
/// shared tick needs to see.
const ATTR: &str = "data-ui-reltime";

pub fn register() {
    // Factory form: called per instance, which is what gives each node its own
    // DOM tag. Registering a *value* def instead would share one tag across every
    // row and the tick would only ever find the first.
    mjs::component_factory(NAME, |_props| {
        let id = format!(
            "ui-rt-{}",
            NEXT_ID.with(|n| {
                let v = n.get() + 1;
                n.set(v);
                v
            })
        );
        def(&id)
    });
}

fn def(id: &str) -> JsValue {
    let mut d = Def::new().named(NAME);
    d.prop_str("at", "").str("tag", id).tree(
        m("span")
            .class("ui-reltime")
            .attr("data-ui-reltime", id)
            // no x-text here — see "The m-js fast-update contract" above
            .attr("title", ""),
    );
    d.method("init", |this, _args| {
        let tag = js::get_str(&this, "tag");
        SCOPES.with(|s| s.borrow_mut().insert(tag.clone(), this.clone()));
        // subscribe, do not schedule: one chain serves every instance on the page
        ensure_tick();
        write_tag(&tag); // first paint now, not up to a second from now
        JsValue::UNDEFINED
    })
    .method("destroy", |this, _args| {
        SCOPES.with(|s| s.borrow_mut().remove(&js::get_str(&this, "tag")));
        JsValue::UNDEFINED
    });
    d.finish()
}

// ---------------------------------------------------------------------------
// formatting
// ---------------------------------------------------------------------------

/// Relative short form, local tz: `2sec ago`, `5min ago`, `3hr ago`, `12day ago`.
///
/// Ported bucket-for-bucket from `ui-reltime.mjs`, including the odd grammar —
/// `3hr ago`, not `3hr ago`/`3 hours ago`. Matching the sibling apps' wording
/// matters more than matching a style guide, and these three boards are meant to
/// read as one product.
///
/// Future instants (a `due` date that has not arrived) read `in 5day`, which is
/// why the sign is captured before the magnitude is bucketed.
pub fn fmt_rel_short(at_ms: f64, now_ms: f64) -> String {
    if !at_ms.is_finite() {
        return "—".to_string();
    }
    let diff = ((now_ms - at_ms) / 1000.0).round();
    let future = diff < 0.0;
    let s = diff.abs();
    let out = if s < 60.0 {
        format!("{}sec", s as i64)
    } else if s < 3600.0 {
        format!("{}min", (s / 60.0).floor() as i64)
    } else if s < 86_400.0 {
        format!("{}hr", (s / 3600.0).floor() as i64)
    } else if s < 86_400.0 * 30.0 {
        format!("{}day", (s / 86_400.0).floor() as i64)
    } else if s < 86_400.0 * 365.0 {
        format!("{}mo", (s / (86_400.0 * 30.0)).floor() as i64)
    } else {
        format!("{}yr", (s / (86_400.0 * 365.0)).floor() as i64)
    };
    if future {
        format!("in {out}")
    } else {
        format!("{out} ago")
    }
}

// ---------------------------------------------------------------------------
// parsing and formatting — pure Rust, deliberately
// ---------------------------------------------------------------------------
//
// Both of these used to call into JS (`Date.parse`, `Date.toLocaleString`).
// They cannot, and the reason is not style: the app's cell renderers run
// **inside** the table store's `borrow_mut()` (`tblstore::rows` holds it for the
// whole materialisation). Any call into JS is a call into a world that can call
// back — m-js drives everything through reactive proxies and component methods
// that read the table right back — and the second that happens the `RefCell`
// is already borrowed and the whole table panics into an empty shell.
// `toLocaleString` did exactly that.
//
// So the render path makes **no** JS calls at all. That also buys native unit
// tests (js_sys panics in a test binary), a deterministic string instead of one
// that changes with the machine's locale, and no per-row-per-second parse.
//
// The shapes covered are the two this app actually stores: a date-only
// `2026-08-22` (what `<input type=date>` and the `due:` shorthand produce) and a
// full `2026-08-17T19:41:32Z` / with-offset timestamp (what the server stamps).

fn digits(v: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let slice = v.get(range)?;
    if slice.is_empty() || !slice.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

/// Howard Hinnant's `days_from_civil`, inlined so the kit stays dependency-free.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`]. Month numbering starts at March inside
/// the algorithm, so the restore is `mp + 3`, not `mp + 1` — getting that wrong
/// shifts every date by two months while still looking like a valid timestamp.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    // one u64 in the middle is where this used to fail to compile: the day-of-era
    // arithmetic is naturally unsigned, but every downstream branch compares and
    // returns i64, so it is widened once, here, rather than at six call sites
    let doe = (z - era * 146_097) as u64;
    let yoe = ((doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365) as i64;
    let y = yoe + era * 400;
    let doy = (doe as i64) - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS[.fff][Z|±hh:mm]` → epoch ms.
///
/// `None` for anything else — an absent value must render as an em-dash, never
/// as `NaNyr ago`, which is the failure that looks like data rather than a bug.
///
/// A date-only value is read as **UTC midnight** (matching `Date.parse`, so the
/// two agree wherever both are used). A timestamp with an offset is normalised to
/// UTC before the epoch arithmetic.
pub fn to_epoch_ms(iso: &str) -> Option<f64> {
    let v = iso.trim();
    if v.len() < 10 {
        return None;
    }
    let (y, mo, d) = (digits(v, 0..4)?, digits(v, 5..7)?, digits(v, 8..10)?);
    // the separators are checked as separators. The first cut of this tested
    // `v.as_bytes()[4].is_ascii_digit()`, i.e. "the dash is a digit", which made
    // **every** valid date return None — and because `fmt_abs` returns "" for
    // None and the cell falls back to an em-dash, the bug would have looked like
    // missing data rather than a broken parser.
    if v.as_bytes()[4] != b'-' || v.as_bytes()[7] != b'-' {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    // mutated only in the offset branch below, which is why `secs` and not `days`
    // is the `mut` one
    let mut secs = 0i64;

    if v.len() > 10 {
        // the date-only forms `2026-08-22Z` are not real; require the separator
        let sep = v.as_bytes()[10];
        if sep != b'T' && sep != b' ' {
            return None;
        }
        let rest = &v[11..];
        let (hh, mm, ss) = (
            digits(rest, 0..2)?,
            digits(rest, 3..5)?,
            digits(rest, 6..8).unwrap_or(0),
        );
        if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) || !(0..=60).contains(&ss) {
            return None;
        }
        secs = hh * 3600 + mm * 60 + ss;
        // everything past the seconds is optional: fractional seconds (accepted
        // and dropped, since `now_iso` never emits them) and a zone, which is
        // either `Z` or a `±hh:mm` offset folded into `secs` so the result is
        // always a UTC instant
        let mut tail = &rest[rest.len().min(8usize)..];
        if let Some(f) = tail.strip_prefix('.') {
            // `char` spelled out: `trim_start_matches` accepts several pattern
            // types, so a bare closure parameter is ambiguous without it
            tail = f.trim_start_matches(|c: char| c.is_ascii_digit());
        }
        if let Some(t) = tail.strip_prefix(['Z', 'z']) {
            if !t.is_empty() {
                return None;
            }
        } else if !tail.is_empty() {
            let sign = tail.as_bytes()[0];
            if sign != b'+' && sign != b'-' {
                return None;
            }
            let oh = digits(tail, 1..3)?;
            let om = digits(tail, 4..6).unwrap_or(0);
            let offset = oh * 3600 + om * 60;
            // a `-05:00` zone means the wall clock read ahead of UTC, so the
            // instant is *later*: subtracting a negative offset adds it back
            secs -= if sign == b'-' { -offset } else { offset };
        }
    }
    Some((days * 86_400 + secs) as f64 * 1000.0)
}

/// The absolute instant for the tooltip, in UTC and stable across machines.
///
/// Deliberately **not** `toLocaleString`: the viewer's locale made the string
/// change with the machine it ran on, and — the reason it could not stay —
/// calling into JS from the render path re-entered the table store and panicked
/// the whole table. The stored values are UTC to begin with, so this is also the
/// more faithful reading of them.
///
/// A date-only value prints its date and no time; a timestamp prints both, so
/// `due` does not claim a deadline happened at midnight.
pub fn fmt_abs(iso: &str) -> String {
    match to_epoch_ms(iso) {
        None => String::new(),
        Some(ms) => {
            let total = (ms / 1000.0).floor() as i64;
            let (days, secs) = (total.div_euclid(86_400), total.rem_euclid(86_400));
            let (y, m, d) = civil_from_days(days);
            let date = format!("{y:04}-{m:02}-{d:02}");
            if iso.trim().len() > 10 {
                format!("{date} {:02}:{:02}:{:02} UTC", secs / 3600, (secs % 3600) / 60, secs % 60)
            } else {
                date
            }
        }
    }
}

/// Start the page's single tick, once per boot.
///
/// This is the whole point of the shared-timer design, and it buys two things.
///
/// **Cost.** Each instance used to call `lifecycle::every` from its own `init()`,
/// so *N* cells meant *N* independent `setTimeout` chains, each performing its own
/// `document.querySelector` every second. On the tasks board that was 6 chains for
/// the date columns alone. Now: one chain, one query, one pass.
///
/// **Consistency.** One `Date.now()` read per tick means every label on screen is
/// bucketed against the *same* instant. With per-instance timers the callbacks
/// drift a few milliseconds apart, and two cells showing the same timestamp could
/// transiently disagree — one on "11sec ago" while its twin had already crossed to
/// "12sec ago". That was a real (if brief) wrongness, not just a perf win.
fn ensure_tick() {
    let token = lifecycle::token();
    if TICK_FOR.with(|t| *t.borrow() == Some(token.clone())) {
        return;
    }
    TICK_FOR.with(|t| *t.borrow_mut() = Some(token.clone()));
    let for_cb = token.clone();
    lifecycle::every(TICK_MS, move || {
        // a hot swap supersedes this boot; ticking on would let a dead instance
        // keep writing into the live DOM
        if !lifecycle::am_current_token(&for_cb) {
            return;
        }
        tick_all();
    });
}

/// One tick for every instance on the page.
fn tick_all() {
    // The DOM is the authority on which instances exist, so it is walked once and
    // the scope map is only ever probed by tag. That also makes the map
    // self-cleaning: a node that is gone is never written, and its scope is
    // dropped below rather than leaking until the next boot.
    let Some(nodes) = js::query_all(&format!("[{ATTR}]")) else {
        return;
    };
    let now = js_sys::Date::now();
    // a NodeList is array-like, not an `Array`, so length and index both come
    // through Reflect — the same reason `tblstore` reads its `querySelectorAll`
    // result this way
    let count = js_sys::Reflect::get(&nodes, &JsValue::from_str("length"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as usize;
    let mut live: Vec<String> = Vec::with_capacity(count);
    for i in 0..count {
        let node = js_sys::Reflect::get(&nodes, &JsValue::from(i as u32))
            .unwrap_or_else(|_| JsValue::UNDEFINED);
        if node.is_null() || node.is_undefined() {
            continue;
        }
        let tag = js::call_string(&node, "getAttribute", &[JsValue::from_str(ATTR)]);
        if tag.is_empty() {
            continue;
        }
        let at = SCOPES.with(|s| {
            s.borrow()
                .get(&tag)
                .map(|scope| js::get_str(scope, "at"))
                .unwrap_or_default()
        });
        write_node(&node, &at, now);
        live.push(tag);
    }
    SCOPES.with(|s| {
        let mut m = s.borrow_mut();
        m.retain(|tag, _| live.iter().any(|l| l == tag));
    });
}

/// The long-form instant for the tooltip: `Saturday, October 17, 2026 at
/// 6:28:07 PM MDT`.
///
/// **This calls into JS, so it must never be called from the table's render
/// path.** The app's cell renderers run inside the table store's `borrow_mut()`,
/// and a JS call from there can come back around through m-js's reactive proxies
/// and re-enter the store — that is precisely the panic that killed the whole
/// table once already (`RefCell already borrowed`). It is safe *here*, as a
/// widget method, because template expressions are evaluated after `rows()` has
/// returned and released the borrow. Call it from the template, never from a
/// `Cell`.
///
/// `Intl` does the work rather than a hand-rolled formatter: weekday names, month
/// names, 12-hour am/pm and the timezone abbreviation are exactly what a locale
/// owns, and this app has no business guessing at them. [`fmt_abs`] stays as the
/// fallback if the call yields nothing.
pub fn fmt_abs_long(iso: &str) -> String {
    let Some(ms) = to_epoch_ms(iso) else {
        return String::new();
    };
    // A **date-only** value has no time and no zone, so it must not be rendered
    // as if it had one. `2026-08-22` parses to UTC midnight, and showing that in
    // the viewer's local zone prints "August 21, 6:00:00 PM" — a deadline a full
    // day early, which for a `due` date is not a cosmetic miss. Dates get a date.
    let date_only = !iso.trim().contains('T') && !iso.trim().contains(' ');
    let opts = js::new_object();
    if date_only {
        // `dateStyle` alone is legal and is the whole answer — but it still has
        // to be read as the **calendar date**, not as a UTC instant viewed from
        // wherever the browser is. `2026-08-22` parses to UTC midnight; formatted
        // in the local zone that is "August 21" six hours before noon, so every
        // deadline in a `due` column would print a day early for anyone west of
        // Greenwich. `timeZone` is the one option ECMA-402 does allow alongside
        // `dateStyle`, and pinning it to UTC makes the weekday and the day agree
        // with the string the user actually typed.
        js::set_str(&opts, "dateStyle", "full");
        js::set_str(&opts, "timeZone", "UTC");
    } else {
        // The obvious way to ask for "Saturday, October 17, 2026 at 6:28:07 PM
        // MDT" is `dateStyle: full, timeStyle: medium, timeZoneName: short` —
        // and it throws `RangeError`. ECMA-402 forbids any other option
        // alongside `dateStyle`/`timeStyle` (only `timeZone` may accompany
        // them), so the zone abbreviation has to be earned by spelling every
        // field out. Verified: the abbreviated form rendered the tooltip label
        // as the literal string "undefined", because a panic in the widget
        // method becomes a JS exception that m-js swallows into `undefined`.
        js::set_str(&opts, "weekday", "long");
        js::set_str(&opts, "year", "numeric");
        js::set_str(&opts, "month", "long");
        js::set_str(&opts, "day", "numeric");
        js::set_str(&opts, "hour", "numeric");
        js::set_str(&opts, "minute", "2-digit");
        js::set_str(&opts, "second", "2-digit");
        js::set_str(&opts, "timeZoneName", "short");
    }
    // `js_sys`'s typed binding is `to_locale_string(locale: &str, options)`, and
    // that `&str` is a trap: the natural "use the default" value is `""`, which
    // is **not** a valid BCP 47 tag, so `Intl` throws `RangeError` and the label
    // comes back empty. The ECMAScript spelling of "default locale" is
    // `undefined`, so the call goes through `js::call` with two explicit
    // arguments instead of the convenience binding.
    let date: JsValue = js_sys::Date::new(&JsValue::from_f64(ms)).into();
    let text = js::call_string(&date, "toLocaleString", &[JsValue::UNDEFINED, opts.into()]);
    let text = text.trim();
    if text.is_empty() {
        fmt_abs(iso)
    } else {
        text.to_string()
    }
}

/// Write one instance's text, given its node and the tick's single `now`.
fn write_node(node: &JsValue, at: &str, now: f64) {
    let text = match to_epoch_ms(at) {
        Some(ms) => fmt_rel_short(ms, now),
        None => "—".to_string(),
    };
    // changed-only: rewriting the same string every second would dirty the DOM and
    // schedule a redraw for a value that did not move — which, with the shared
    // tick, would now be a redraw caused by *every* instance rather than one
    if js::get_str(node, "textContent") != text {
        js::set_text(node, &text);
    }
}

/// Single-instance write, used for the first paint before the tick next fires.
fn write_tag(tag: &str) {
    let at = SCOPES.with(|s| {
        s.borrow()
            .get(tag)
            .map(|scope| js::get_str(scope, "at"))
            .unwrap_or_default()
    });
    let Some(node) = js::query(&format!("[{ATTR}=\"{tag}\"]")) else {
        return; // not mounted yet
    };
    write_node(&node, &at, js_sys::Date::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f64 = 1000.0;
    const M: f64 = 60_000.0;
    const H: f64 = 3_600_000.0;
    const D: f64 = 86_400_000.0;

    #[test]
    fn buckets_match_the_canonical_component() {
        let now = 1_700_000_000_000.0;
        assert_eq!(fmt_rel_short(now, now), "0sec ago");
        assert_eq!(fmt_rel_short(now - 2.0 * S, now), "2sec ago");
        assert_eq!(fmt_rel_short(now - 59.4 * S, now), "59sec ago");
        assert_eq!(fmt_rel_short(now - 2.0 * M, now), "2min ago");
        assert_eq!(fmt_rel_short(now - 90.0 * M, now), "1hr ago", "59min rounds into the hour bucket");
        assert_eq!(fmt_rel_short(now - 3.0 * H, now), "3hr ago");
        assert_eq!(fmt_rel_short(now - 12.0 * D, now), "12day ago");
        assert_eq!(fmt_rel_short(now - 45.0 * D, now), "1mo ago");
        assert_eq!(fmt_rel_short(now - 400.0 * D, now), "1yr ago");
    }

    /// A future instant is the whole point of the `due` column, so the sign has
    /// to be taken before the magnitude is bucketed, not after.
    #[test]
    fn the_future_reads_as_in_not_ago() {
        let now = 1_700_000_000_000.0;
        assert_eq!(fmt_rel_short(now + 2.0 * S, now), "in 2sec");
        assert_eq!(fmt_rel_short(now + 5.0 * D, now), "in 5day");
        assert!(!fmt_rel_short(now + 5.0 * D, now).contains("ago"));
    }

    #[test]
    fn the_bucket_boundaries_are_the_documented_ones() {
        let now = 0.0;
        // one second either side of every threshold changes the unit, not the text
        // 59.4, not 59.99: the seconds value is *rounded* before it is bucketed
        // (same as the canonical `Math.round((nowMs - atMs) / 1000)`), so 59.99
        // honestly becomes 60 and belongs to the minute bucket. My first draft
        // of this test asserted "59sec ago" and the test, not the code, was wrong.
        assert_eq!(fmt_rel_short(now - 59.4 * S, now), "59sec ago");
        assert_eq!(fmt_rel_short(now - 59.6 * S, now), "1min ago", "rounds up into the next bucket");
        assert_eq!(fmt_rel_short(now - 60.0 * S, now), "1min ago");
        assert_eq!(fmt_rel_short(now - 3600.0 * S, now), "1hr ago");
        assert_eq!(fmt_rel_short(now - 86_400.0 * S, now), "1day ago");
        assert_eq!(fmt_rel_short(now - 86_400.0 * 30.0 * S, now), "1mo ago");
        assert_eq!(fmt_rel_short(now - 86_400.0 * 365.0 * S, now), "1yr ago");
    }

    /// An unparseable or missing date must not become `NaNyr ago` — the failure
    /// mode that looks like data rather than a bug.
    /// The pure half: a NaN instant must render as an em-dash, never `NaNyr ago`
    /// — the failure mode that looks like data rather than a bug.
    #[test]
    fn a_nan_instant_is_an_em_dash() {
        assert_eq!(fmt_rel_short(f64::NAN, 0.0), "—");
        assert_eq!(fmt_rel_short(f64::INFINITY, 0.0), "—");
    }

    /// These run natively now. They could not when parsing went through
    /// `Date.parse` — js_sys calls panic in a test binary — which is also why
    /// nothing here was tested before. Pure Rust bought the tests, not just the
    /// absence of the re-entrancy panic.
    #[test]
    fn absent_and_junk_dates_parse_to_nothing() {
        assert!(to_epoch_ms("").is_none());
        assert!(to_epoch_ms("   ").is_none());
        assert!(to_epoch_ms("not a date").is_none());
        assert!(to_epoch_ms("2026-08-17T19:41:32Z").is_some());
        // date-only, the shape a `due` value has
        assert!(to_epoch_ms("2026-08-22").is_some());
    }

    /// The exact strings this app stores, both directions.
    #[test]
    fn the_two_stored_shapes_parse_and_format() {
        // the server's own stamp, from `now_iso()`
        let full = "2026-08-17T19:41:32Z";
        assert_eq!(to_epoch_ms(full), Some(1_786_995_692_000.0));
        assert_eq!(fmt_abs(full), "2026-08-17 19:41:32 UTC");

        // a date-only value, what `<input type=date>` and `due:` produce: read as
        // UTC midnight, and printed WITHOUT a time, so a deadline is not claimed
        // to have happened at midnight
        let day = "2026-08-22";
        assert_eq!(to_epoch_ms(day), Some(1_787_356_800_000.0));
        assert_eq!(fmt_abs(day), "2026-08-22");
    }

    /// Round trip: format what you parsed, parse what you formatted. This is the
    /// test that would have caught the `mp + 1` month bug on the kit's own copy
    /// of the algorithm, which is why it is here and not just in the server.
    #[test]
    fn epoch_and_civil_agree_in_both_directions() {
        for days in -40_000i64..40_000 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "day {days} printed {y}-{m}-{d}");
        }
    }

    /// A zone offset is folded into the instant, not ignored: the same wall
    /// clock in `+02:00` is two hours earlier in UTC than in `Z`.
    #[test]
    fn an_offset_shifts_the_instant() {
        let z = to_epoch_ms("2026-08-17T12:00:00Z").unwrap();
        let plus = to_epoch_ms("2026-08-17T12:00:00+02:00").unwrap();
        let minus = to_epoch_ms("2026-08-17T12:00:00-05:00").unwrap();
        assert_eq!(plus, z - 2.0 * 3_600_000.0, "+02:00 is an earlier instant");
        assert_eq!(minus, z + 5.0 * 3_600_000.0, "-05:00 is a later one");
        assert_eq!(fmt_abs("2026-08-17T12:00:00+02:00"), "2026-08-17 10:00:00 UTC");
        // fractional seconds are tolerated, since callers may hand over a
        // timestamp this module did not mint
        assert!(to_epoch_ms("2026-08-17T12:00:00.123Z").is_some());
        assert!(to_epoch_ms("2026-13-01").is_none(), "month 13 is not a date");
        assert!(to_epoch_ms("2026-08-32T00:00:00Z").is_none(), "nor is day 32");
        assert!(to_epoch_ms("2026/08/17").is_none(), "separators must be real");
    }
}
