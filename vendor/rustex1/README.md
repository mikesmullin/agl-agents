# rustex1

A prototype that answers one question: **what does hot reload feel like when the
backend is Rust and the frontend is Rust too?**

## The verdict: the stack works

This started as a bet and it is now a **working proof that Rust + wasm is a
viable stack for building web applications** — not just for the compute, but for
the day-to-day authoring experience. One language, one build tool (`cargo`), one
process (`rustex1-server`) serving the app, watching the sources, compiling the
frontend, and pushing the result into the browser. And the reload loop holds up
against the JS toolchain it replaces:

- **0.8–1.8s** from saving a `.rs` file to the new UI painting, measured, no
  page reload, and the state you were looking at is still there.
- Editing a Rust file changes **copy, props, templates, logic and data** alike —
  the whole surface a frontend developer touches.
- The three tiers (wasm swap / stylesheet swap / backend re-exec) all work, and
  failures in each are reported *in the browser* rather than only in a terminal.
- The parts that normally bite you in Rust-wasm — a dead `requestAnimationFrame`
  loop outliving its instance, a cached `.wasm` silently hot-reloading the
  previous build, a blank element from a typo'd store — are handled, and one of
  them (bad directives) got its own boot-time linter.

What that means practically: **you can build a real web app this way today.**
Nothing on the client is authored in JavaScript except two thin bridge files
(`web/app.mjs` boots the wasm, `web/hot.mjs` speaks m-js's hot-reload protocol,
~350 lines together) plus the m-js CDN bundle doing the rendering. Everything a
product developer writes — components, templates, state, routes, timers,
networking — is Rust.

What a prototype is still not: a production story. There is no test harness, no
CI, no release/bundling pipeline, no CSS or asset pipeline beyond a stylesheet,
the dev server is single-connection-per-thread and dev-only by design, and a
`.rs` save rebuilds the tree (scroll position and input focus inside `#app` do
not survive it — a `.css` save loses nothing). Those are the honest edges, not
blockers for the claim above.

Same idea as `bun run dev` + m-js, with the client rewritten in Rust:

```
   edit app/**.rs ──► cargo build --target wasm32 ──► wasm-bindgen ──► web/pkg/*
        │                                                                  │
        │                        /__m_hmr  (one websocket)                 │
        └── rust dev server ◄──────────────────────────────────────────────┘
                                    │
                                    ▼
                     {type:"change", path:"/pkg/rustex1_app.js"}
                                    │
                                    ▼
                     m-js hot client → window.__M_BOOT__(t)
                                    │
                                    ▼
                     new wasm instance → M.unmount() → M.mount('#app')
                     stores on `window` survive, the page never reloads
```

## Run it

```bash
cd tmp/rustex1
./dev.sh            # → http://127.0.0.1:3001
./dev.sh 4000       # other port
./dev.sh --rebuild  # force a fresh wasm build
```

`dev.sh` installs the pinned `wasm-bindgen` CLI into `tools/` and the
`wasm32-unknown-unknown` target if either is missing. The server builds the
frontend itself before it answers the first request.

## The three reload tiers

| you edit                | cost                                   | what you see                                        |
|-------------------------|----------------------------------------|-----------------------------------------------------|
| `app/src/demo.rs`       | cargo + wasm-bindgen (~0.8–2s)         | text/props/templates change, **no reload, state kept** |
| any other `app/**.rs`, or the kit's sources | same            | components re-register, page re-diffs               |
| `web/styles.css`        | none                                   | `<link>` cache-busted in place, zero redraws        |
| `web/*.mjs`             | module re-import                       | `__M_BOOT__(t)` re-runs                             |
| `web/index.html`        | none                                   | full reload (the document shell changed)            |
| `server/src/**.rs`      | cargo build (~1.6s)                    | process re-execs itself, client reconnects + reloads |

Compile errors do **not** kill the dev server, and they do not cover the page
either: cargo's stderr is pushed over the socket, mirrored into
`window.__HMR__.error` and the browser console, and the last good build keeps
running underneath. The UI-side signal is the one-second flash line — the build
output itself belongs to the terminal running cargo, and that is where you read
it. The red panel is the **template linter's** (`kit/src/lint.rs`), which has no
terminal of its own.

## Layout

Four crates: two libraries to depend on, and the demo app that depends on them.

```
devserver/           rustex1-devserver — the reusable backend (lib)
  src/lib.rs           Server::new(config).api("/api/x", h).run()
  src/config.rs        rustex1.json: name, port, packages, watch dirs, static roots
  src/http.rs          multi-root static + MIME + SPA fallback, built-in JSON API
  src/hub.rs           /__m_hmr websocket hub (broadcast, heartbeat, poll loop)
  src/watcher.rs       notify → debounce → classify → build → broadcast
  src/build.rs         cargo + wasm-bindgen, build log, self-restart
  src/state.rs         shared state, build records, logging

kit/                 rustex1-kit — the reusable frontend (lib, wasm)
  src/lib.rs           App trait + boot(): stores → widgets → routes → lint → mount
  src/h.rs             m() hyperscript — templates as Rust trees, not markup
  src/lint.rs          directive linter + self-test (7 rules)
  src/mjs.rs           M.* / Router.* bindings + the Def builder
  src/js.rs            forgiving js_sys wrappers (reflect, call, closures, DOM sugar)
  src/hooks.rs         the JS bridge handed to boot() (this-binding, overlay, stats)
  src/lifecycle.rs     boot token + rAF/setTimeout that stop when superseded
  src/session.rs       the `session` + `toasts` stores (survive a swap)
  src/api.rs           /api/status + /api/builds polling, rebuild/restart calls
  src/live.rs          5 Hz mirror of window.__HMR__ into $store.session
  src/ui.rs            widget catalog: badges, buttons, stats, gauge, toasts,
                       pipeline panel, build feed
  web/app.mjs          CDN import of m-js, wasm init, window.__M_BOOT__
  web/hot.mjs          HMR client (m-js protocol + build/restart/bootId extras)
  web/kit.css          the skin any app can override
  web/vendor/phosphor/ phosphor icon webfont (from the unibox3 bundle)
  build.rs             bakes rustc version + build stamp into the wasm

app/                 the rustex1 demo frontend — a *consumer* of the kit
  src/lib.rs           impl App + the #[wasm_bindgen] entry points
  src/pages.rs         routes: /, /storybook, /protocol, /counter/:n
  src/components.rs    navBar, counterCard, hintStrip (the app-specific three)
  src/store.rs         counter + meters (state a swap must not lose)
  src/live.rs          the gauge frame loop: ONE setAttribute per frame
  src/demo.rs          ← constants to poke at while demoing

server/              the rustex1 demo backend: Server::new(…).api(…).run()
web/                 this app's static root: index.html + styles.css (overrides)
docs/RUST_M_JS.md    contributor guide for everything in kit/
rustex1.json         which packages to build, ports, watch and static roots
```

## Using rustex1 as a base dependency

This repo is designed to be vendored, not copied. `tmp/tasks1` is a real second
app doing exactly that; start there.

**1 · add it as a submodule**

```bash
mkdir myapp && cd myapp && git init
git submodule add ../rustex1 vendor/rustex1     # or any git URL
git submodule update --init --recursive
```

**2 · depend on the two libraries**

```toml
# Cargo.toml — members = ["app", "server"], plus this profile stanza, which the
# dev server needs because it builds the wasm half from *your* workspace root:
#
# [profile.wasm]
# inherits = "release"
# opt-level = "s"
# debug = false
# strip = "debuginfo"
# codegen-units = 16

# app/Cargo.toml
rustex1-kit = { path = "../vendor/rustex1/kit" }
wasm-bindgen = "=0.2.129"
js-sys = "=0.3.106"

# server/Cargo.toml
rustex1-devserver = { path = "../vendor/rustex1/devserver" }
serde_json = "1"
```

**3 · describe the app** — `rustex1.json`, defaults derived from `name`:

```json
{
  "name": "myapp",
  "port": 3020,
  "app":    { "package": "myapp-app", "lib": "myapp_app", "watch": ["app"] },
  "server": { "package": "myapp-server", "watch": ["server"] },
  "static": ["web"]
}
```

**4 · write the app.** The backend is configuration plus routes — any method, and
`req.json()` carries the body for writes:

```rust
use rustex1_devserver::{args_to_config, Config, Response, Server};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (config, rebuild) = args_to_config(&args, Config::discover(&std::env::current_dir().unwrap()));
    let mut server = Server::new(config)
        // a bare serde_json::Value is a 200…
        .api("/api/hello", |state, _req| json!({ "message": "hello from rust" }))
        // …or return a Response for another status
        .api("/api/items", |_state, req| match req.method.as_str() {
            "POST" => match req.json() {
                Some(body) => Response::created(json!({ "ok": true, "created": body })),
                None => Response::bad_request("expected a JSON body"),
            },
            "DELETE" => Response::not_found("nope"),
            _ => json!({ "ok": true }),
        });
    if rebuild { server = server.rebuild_on_start(); }
    server.run();
}
```

Registered routes also see `req.query` (with `req.param("key")`), and the body is
always drained so a keep-alive socket never RSTs a response.

and the frontend is one `App` implementation:

```rust
use rustex1_kit::{self as kit, m, lit, Def, Runtime, App};
use wasm_bindgen::prelude::*;

struct MyApp;

impl App for MyApp {
    fn register_stores(&self, rt: &mut Runtime)   { /* M.store definitions */ }
    fn register_components(&self, rt: &mut Runtime) { /* optional additions */ }
    fn register_routes(&self, rt: &mut Runtime) {
        rt.route("/", "myapp", |_| {
            let mut d = Def::new().named("route /");
            d.tree(m("main").class("page").kids(vec![
                m("h1").child(lit("hello, world")),
                // kit widgets are pre-registered:
                kit::Node::host("uiBadge", &[("tone","ok"),("label","live"),("icon","check-circle")]),
                kit::Node::host("buildFeed", &[]),
            ]));
            d.finish()
        });
    }
    fn started(&self, rt: &mut Runtime) { rt.every(1000.0, || { /* low-frequency work */ }); }
}

#[wasm_bindgen]
pub fn boot(m: JsValue, router: JsValue, hooks: JsValue, opts: JsValue) -> Result<(), JsValue> {
    kit::boot(m, router, hooks, opts, &MyApp)
}
```

`web/` needs only an `index.html` with `<div id="app"></div>`, a
`<script type="module" data-hmr-entry src="/app.mjs">` and links to `/kit.css`
then your own `/styles.css`. Everything else — the splash, the error box, the
template-lint overlay, `window.__M_BOOT__`, the websocket client — is created by
the kit's `app.mjs` / `hot.mjs`, which your static root **shadows** if you ever
want to replace it (first root wins, kit root last).

**What you get for free:** the three hot-reload tiers, the template-lint overlay
(rust compile errors only flash + log — see [The three reload tiers](#the-three-reload-tiers)),
`/api/status` + `/api/builds` + `/api/rebuild` +
`/api/restart`, the `session`/`toasts` stores, the seven kit widgets (including
a working HMR status panel and build log), `window.__HMR__` telemetry mirrored
into stores, boot-token-safe loops, and `dumpTemplate` / `lintReport` /
`debugState` / `buildInfo` on the console.

**Keeping the kit fresh:** `git submodule update --remote vendor/rustex1` and
rebuild — nothing to copy, and any file you shadowed in `web/` stays yours.

**New-app checklist** (compare with `tmp/tasks1`):

- [ ] `git submodule add <rustex1> vendor/rustex1`
- [ ] workspace `members = ["app","server"]` **plus the `[profile.wasm]` stanza**
- [ ] `app` = cdylib depending on `rustex1-kit`; `server` = bin depending on `rustex1-devserver`
- [ ] `rustex1.json` with `name`, `port`, package/lib names, watch dirs
- [ ] `web/index.html` (app div + module script + `/kit.css` then `/styles.css`)
- [ ] `dev.sh` mirroring [tasks1/dev.sh](../tasks1/dev.sh) (target + wasm-bindgen bootstrap)
- [ ] add `target/`, `web/pkg/`, `tools/` to `.gitignore`

## API

| route | purpose |
|-------|---------|
| `GET /api/status` | uptime, request count, ws clients, current build, last error |
| `GET /api/builds?limit=n` | ring buffer of builds with duration, trigger and stderr |
| `POST /api/rebuild` | run the wasm build on demand (the Home page button) |
| `POST /api/restart` | re-exec the server — backend hot reload from the browser |
| `GET /__m_hmr` | the websocket |

## Wire protocol

Stock m-js speaks two messages; this server adds three more that a stock client
simply ignores:

```jsonc
{ "type": "connected", "bootId": "17d1-9f3a…", "server": "rustex1", "clients": 1 }
{ "type": "change",    "path": "/pkg/rustex1_app.js" }

{ "type": "build",     "phase": "start", "seq": 12, "target": "wasm", "trigger": ["app/src/demo.rs"] }
{ "type": "build",     "phase": "ok",    "seq": 12, "target": "wasm", "ms": 1343 }
{ "type": "build",     "phase": "error", "seq": 12, "target": "wasm", "error": "error[E0308]: …" }
{ "type": "restart",   "target": "server" }
```

A changed `bootId` on reconnect means the process was replaced → one full
reload. Everything else stays hot.

## Reading the seams

Everything this project puts **on top of** m-js is documented in
[`docs/RUST_M_JS.md`](docs/RUST_M_JS.md) — read that before touching `app/`.
Short version of the four additions:

| we added | file | because m-js… |
|---|---|---|
| `m()` hyperscript | `app/src/h.rs` | has no hyperscript (`import m from 'm-js'` is the `M` namespace), and its HTML-string templates are **repaired, not rejected**, by the browser's parser — mis-closed tags ship silently |
| `Def` + `M.*` bindings | `app/src/mjs.rs` | is a JS API, and a wasm closure has no `this` — so `hooks.bind` forwards it — every Rust method takes the scope object first, then an args array |
| boot-token lifecycle | `app/src/lifecycle.rs` | nothing in it knows a new wasm instance replaced the old; loops must self-retire via the `window.__RUSTEX1__` token or they fight over the same node |
| template linter | `app/src/lint.rs` | swallows five kinds of wrong directive — typo'd store, un-compilable expression, unknown `x-component`, malformed `x-for`, misspelled `x-` name — all of which render "nothing" with no error |

The doc covers the `m()` API in full (attributes, typed directives, `Node::host`,
`icon`/`icon_dyn`, escaping, void elements, why output is pretty-printed), the
store rules for what survives a swap, the one-attribute-per-frame pattern, a
walkthrough for adding a widget or a route, and the console toolbox
(`dumpTemplate`, `lintReport`, `debugState`, `__HMR__`).

## Demo script

1. `./dev.sh`, open <http://127.0.0.1:3001>, click **+1 / +10** a few times.
2. Edit `app/src/demo.rs` → `HEADLINE`, `COUNTER_LABEL`, `GAUGE_LABEL`,
   `BUILD_BUDGET_MS`, `HINTS` → save. Watch the flash line, the build log row,
   the new copy — and the unchanged counter.
3. Edit a template in `app/src/components.rs` → save → markup changes in place
   (everything is an `m()` tree now; `h.rs` is the serializer).
4. Break a directive on purpose — `x-tex="label"`, `$store.sesion.x`,
   `x-for="items"`, `x-text="count +"` — save, and read the finding. Cargo is
   happy (it is only a string), which is exactly why this layer exists.
5. Change `--accent` in `web/styles.css` → save → stylesheet swaps, no redraw at all.
6. Break Rust on purpose (`pub fn broken( {`) → one red flash line, the text in
   the console and `__HMR__.error`, cargo's own output in the terminal, page
   keeps running on the last good build → fix it.
7. Edit `server/src/http.rs` (add a route, change JSON) → save → the backend
   rebuilds, re-execs, the browser reconnects and reloads by itself.
8. Hit the **rebuild wasm** button on Home → that's `POST /api/rebuild` → cargo → push.

## Notes and known edges

- **Hot swap = unmount + mount.** m-js keeps live component instances alive when
  a definition is re-registered ("existing instances keep their state"), which
  also means they keep their *old* template string. A swap therefore unmounts
  and re-mounts the root: the tree is rebuilt, stores are not. Scroll position
  and input focus inside the app region are lost on a `.rs` save — a `.css` save
  loses nothing.
- **Hidden tabs stall the frame loop.** `requestAnimationFrame` (and m-js's
  frame-scheduled redraw) does not run in a background tab; the timers and the
  store keep going, and the paint catches up when the tab is shown again.
- **A server restart can truncate an in-flight `.wasm` response**, which Chrome
  reports as "WebAssembly compilation aborted: Network error". `app.mjs` retries
  the boot a few times before showing anything alarming.
- Everything is `Cache-Control: no-store` plus cache-busting query strings —
  a cached wasm would silently "hot reload" the previous build.
- Requests are single-threaded per connection and `Connection: close`; this is a
  dev server, not a web server.
- `web/pkg/`, `target/` and `tools/` are generated and gitignored.
