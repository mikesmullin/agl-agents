//! ## Edit these and save.
//!
//! This file exists to be poked at. Change a value, hit `:w`, and the page
//! should change under you — no reload, no lost state — because the watcher
//! rebuilds `app/**.rs` into wasm and the m-js hot client calls
//! `window.__M_BOOT__(t)` again.

/// Big headline on the Home page.
pub const HEADLINE: &str = "Rust backend · Rust wasm frontend — edited live";

/// One line under the headline. Try a different sentence.
pub const SUBHEAD: &str = "hot reload over one websocket, one m-js diff, zero page reloads";

/// Shown next to the live gauge.
pub const GAUGE_LABEL: &str = "rebuild pressure (live)";

/// What the counter button says.
pub const COUNTER_LABEL: &str = "add to the store";

/// How many increments one click applies.
pub const COUNTER_STEP: f64 = 1.0;

/// A rebuild slower than this is reported as "slow" (the gauge turns amber).
pub const BUILD_BUDGET_MS: f64 = 3000.0;

/// The toast the page raises once per boot.
pub const BOOT_TOAST: &str = "wasm booted — rust side is live";

/// Extra copy rendered in the "how it works" strip on Home.
pub const HINTS: &[&str] = &[
    "app/**.rs → cargo + wasm-bindgen → /pkg → __M_BOOT__(t)",
    "web/**.css → cache-busted <link>, no redraw at all",
    "server/**.rs → cargo build → process re-exec → full reload",
    "$store.counter is on window, so it outlives every swap",
];
