# Rust ⇄ m-js — contributor guide

Everything in this repo that is **not** m-js, explained for someone opening
`app/` for the first time.

m-js is loaded from the CDN and used as-is — no fork, no patch, no local copy:

```js
import M, { Router } from 'https://mikesmullin.github.io/m-js/dist/m.min.js'
```

On top of it this project adds four things, and each one exists because of a
specific gap. Read the gap first; the API will then feel obvious rather than
arbitrary.

| we added | file | because m-js… |
|---|---|---|
| `m()` hyperscript | `app/src/h.rs` | has no hyperscript at all, and its templates are HTML strings that the **browser's parser silently repairs** |
| `Def` + `M.*` bindings | `app/src/mjs.rs` | is a JS API; Rust needs reflection-shaped helpers, and its closures can't see `this` |
| boot-token lifecycle | `app/src/lifecycle.rs` | nothing in it knows a *new wasm instance* just replaced the old one, so zombie `rAF` loops are your problem |
| template linter | `app/src/lint.rs` | swallows bad directives: five distinct mistakes all render "nothing" with no error |

Plus the pieces that aren't m-js related: the dev server (`server/`), the HMR
client (`web/hot.mjs`), the entry (`web/app.mjs`), stores (`app/src/store.rs`),
widgets (`app/src/components.rs`), routes (`app/src/pages.rs`).

---

## The boot sequence, in one screen

`web/app.mjs` owns it; `app/src/lib.rs` is what it calls.

```
app.mjs                         lib.rs::boot(M, Router, hooks, {bootId})
──────                          ────────────────────────────────────────
import M from CDN                    install_panic_hook
import('./pkg/rustex1_app.js?t=t')   mjs::install(M, Router)     ← the namespace lives here
init({module_or_path: wasm?t})       hooks::install(hooks)       ← bind / overlay / stats
                                     lifecycle::claim(token)     ← window.__RUSTEX1__ = token
                                     store::register()           ← M.store, data survives swaps
                                     components::register()      ← M.component × 10
                                     pages::register()           ← Router.register × 4
                                     lint::run()                 ← 7 rules over recorded trees
                                     if superseded: M.unmount()
                                     M.mount('#app')
                                     api::spawn_all()            ← fetch poll, self-rescheduling
                                     live::spawn()               ← rAF loop, one attr per frame
```

`window.__M_BOOT__` is the hook m-js's hot-reload protocol calls; here it is
`enqueueBoot(t)`, which serialises boots so a burst of file events can't
interleave two mounts.

**Why `unmount()` then `mount()` on a swap.** Re-registering a component updates
the registry, but m-js keeps live instances alive — and `widgetTemplate()` reads
`scope.template` first, which the old instance inherited from the *old*
definition. So without a teardown, an edited template never repaints. Unmounting
rebuilds the tree from the new definitions; stores live on `window` and are
untouched. Consequence worth knowing: a `.rs` save loses scroll position and
input focus inside `#app`. A `.css` save loses nothing.

---

## 1 · `m()` — templates as Rust trees (`app/src/h.rs`)

### The gap

m-js v3 has **no hyperscript**. Despite the name, `import m from 'm-js'` gives
you the `M` namespace object; v2's `M.html(tag, attrs, children)` never made the
jump. Every v3 template is an HTML string, and `parse.js` hands it to the
browser: `<template>.innerHTML`, walk once into an AST, drop the DOM.

The browser's parser does not reject mistakes. It repairs them:

```text
  <div><span x-text="one"></div><p x-text="two"></p></div>
  → [ div > span, p ]      // no error, no warning — the span quietly left the div
```

That is an entire bug class that ships: mis-nested markup that renders
*plausibly wrong*. `h.rs` deletes the class by making it unwriteable.

### What it is

A tiny builder that **emits the template string m-js expects**. Nothing else in
the system changes — same parser, same once-per-string AST compile, all
directives intact.

```rust
use crate::h::{m, lit, Node};

m("span").class("badge").bind("class", "tone").kids(vec![
    m("i").show("icon").bind("class", "'ph ph-' + icon"),
    m("span").text("label"),
])
// <span class="badge" :class="tone">
//   <i x-show="icon" :class="'ph ph-' + icon"></i>
//   <span x-text="label"></span>
// </span>
```

You can check what any component was handed, at runtime:

```js
__RUSTEX1__MOD.dumpTemplate('gaugeBar')
```

### Method reference

Entry points:

| call | meaning |
|---|---|
| `m("tag")` | an element |
| `lit("text")` | a text node (escaped at emit) |
| `frag(vec![…])` | several roots → one template string |

Attributes (last call wins for a repeated name, so a builder can never emit
duplicate attributes — HTML would keep only the first and say nothing):

| call | emits |
|---|---|
| `.at(&[("type","button"),("role","img")])` | literal attrs, the `m('i', {…})` shape |
| `.attr(k, v)` | one literal attr |
| `.class(c)` | `class="c"` |
| `.bind(name, expr)` | `:name="expr"` — distinct from `.attr`, so `class="badge"` + `:class="tone"` coexist as m-js expects |
| `.on(event, expr)` | `@event="expr"` — **any** DOM event name; m-js has no whitelist, so `@dragover.prevent` and `@drop.prevent` are as available as `@click` (§"Known m-js behaviours") |

Directives (typed, so you rarely spell `x-` yourself):

| call | emits |
|---|---|
| `.text(expr)` | `x-text` |
| `.inner_html(expr)` | `x-html` |
| `.show(expr)` | `x-show` |
| `.iff(expr)` | `x-if` |
| `.for_each("t in items")` | `x-for` — pair with `.key()` |
| `.key(expr)` | `:key` |
| `.model(expr)` | `x-model` |
| `.kref(name)` | `x-ref` |
| `.init(expr)` / `.effect(expr)` | `x-init` / `x-effect` |
| `.widget("uiBadge")` | `x-component` |
| `.ignore()` | `x-ignore` |

Sugar and children:

| call | emits |
|---|---|
| `Node::icon("hammer")` | `<i class="ph ph-hammer"></i>` — **check the glyph exists first:** <br>`grep -c '\.ph-hammer:before' kit/web/vendor/phosphor/phosphor.css` |
| `Node::icon_dyn("r.icon")` | `<i :class="'ph ph-' + r.icon"></i>` |
| `Node::host("uiStat", &[("label","uptime")])` | `<span x-component="uiStat" label="uptime"></span>` |
| `Node::host_bound("uiStat", &[("value","$store.x")])` | …with `:value="$store.x"` |
| `.child(node)` / `.kids(vec![…])` | children (`kids` extends, so it chains) |
| `.into_html()` | the template string |
| `.walk(closure)` | depth-first visit over the subtree — what the linter reads |

### Five rules that will bite you

1. **Expressions are JS source in Rust strings.** `.text("count +")` compiles in
   Rust and produces a blank element in the DOM. That is what the linter is for
   (§4) — always read `template lint N/N` in the footer after a change.
2. **Output is pretty-printed and the newlines are load-bearing.** They are the
   whitespace text nodes between inline elements, which is what the old
   multi-line raw strings produced too. Compacting the serializer visually glues
   buttons together. Text-only elements stay on one line.
3. **Void elements emit no closing tag** (`input`, `img`, `br`, … from the `VOID`
   list). Everything else always gets one, closed, in the right place — by
   construction.
4. **`class` and `:class` both work.** m-js merges them; keep the static classes
   on `.class()` and the reactive ones on `.bind("class", …)`.
5. **An icon can fail silently in two ways.** `Node::icon_dyn(e)` emits
   `:class="'ph ph-' + (e)"`, so an un-parenthesised ternary parses as
   `('ph ph-' + cond) ? 'a' : 'b'` — a truthy string, so the class lands as a
   bare `caret-right` **with no `ph ph-` prefix**. And a name that is simply not
   in the vendored subset (`grip`, for one) produces a class that matches no
   rule. Both give you an invisible element that still occupies space and still
   takes the click, which a `textContent` probe reads as fine. Always
   `grep -c '\.ph-<name>:before' kit/web/vendor/phosphor/phosphor.css` before
   using a glyph, and parenthesise every conditional one.

### SVG works

`gaugeBar` is the proof — `viewBox` and `preserveAspectRatio` keep their casing
because the browser's parser is doing foreign-content handling exactly as it
would for a literal template:

```rust
m("svg").at(&[("class","gauge-svg"),("viewBox","0 0 240 10"),("preserveAspectRatio","none")])
    .kids(vec![
        m("rect").at(&[("class","gauge-track"),("width","240"),("height","10"),("rx","5")]),
        m("rect").at(&[("class","gauge-fill"),("data-gauge","pipeline"),("width","0")]),
    ])
```

`data-gauge="pipeline"` is the handshake with the frame loop — see §5.

---

## 2 · `Def` — component and store definitions (`app/src/mjs.rs`)

A widget is a plain JS object handed to `M.component(name, def)`. `Def` builds
it.

```rust
fn ui_badge() {
    let mut d = Def::new().named("uiBadge");          // label = better lint messages
    d.prop_str("tone", "idle")                        // declared prop + default
     .prop_str("label", "—")
     .prop_str("icon", "")
     .prop_bool("pulse", false)
     .tree(m("span").class("badge").bind("class","tone").kids(vec![ /* … */ ]));
    mjs::component("uiBadge", &d.finish());
}
```

| call | purpose |
|---|---|
| `Def::new()` / `.named(label)` | start; the label is what the linter prints |
| `.prop_str/.prop_num/.prop_bool/.prop` | **declared** props — copied from the host element on every redraw, and always given a default |
| `.str/.num/.bool/.val` | plain state (not host-provided) |
| `.tree(node)` / `.trees(nodes)` | the template, hyperscript (also records it for the linter) |
| `.template(&str)` | verbatim markup, bypassing the serializer — nothing uses it today; prefer `.tree` |
| `.method(name, f)` | `this`-bound method; Rust sees `(scope, args)` |
| `.action(name, f)` | same, for side-effect-only methods |
| `.finish()` | attach `props: [...]` and hand back the object |

Give **every declared prop a default**. `Object.create(def)` means a prop the
host omits must still resolve, and m-js's own backfill leaves it `undefined`
where a template binding can silently misbehave.

### Reading a boolean prop back: `js::flag`, never `js::get_bool`

`prop_bool("can-export", true)` declares the prop, but the **host can only write a
string**, and `js::get_bool` is `JsValue::is_truthy` — so `can-export="false"` is
the four-character string `"false"`, which is truthy, and the prop reads as **on**
no matter what the author wrote. A present-but-valueless `<div can-export>` arrives
as `""`, which by HTML convention also means on, and the two cases are
indistinguishable to a truthiness test.

```rust
// wrong: ="false" reads as true
JsValue::from(js::get_bool(&this, "can-export"))
// right: the fallback is the prop's declared default
JsValue::from_bool(js::flag(&this, "can-export", true))
```

`js::flag` is m-js's own `coerceFlag` ported (`false`/`"false"`/`"0"`/`"no"`/`"off"`
→ off, `true`/`"true"`/`""` → on, absent → default, anything else → JS truthiness,
so a bound `:can-export="expr"` still gets a real boolean). This was not
hypothetical: **all eleven** boolean props on the tasks1 table widget had the bug,
so `can-export="false"` had never once worked.

The fix there was a table — `const FLAG_PROPS: &[(&str, &str, bool)]` of
`(kebab attribute, camel getter, default)`, consumed by one loop that declares each
prop and generates its getter. Names *and* defaults are needed twice (once to
declare, once to read), and two copies of a default is a default that drifts. See
`tasks1/app/src/table2/mod.rs`.

`Def` is also how stores are declared — same builder, no `props`, registered
with `mjs::store(name, &d.finish())`.

### The `this` bridge

A wasm-exported closure has no notion of a JS receiver, and wasm-bindgen has no
variadic closures (`FnMut(&[JsValue])` doesn't implement the trait). So `app.mjs`
hands `boot()` a bridge — the whole of it:

```js
const hooks = {
  // wasm closures have fixed arity, so arguments travel as a real Array and
  // every Rust method is written |this, args| { … }.
  bind: (fn) =>
    function bound(...args) {
      return fn(this, args)
    },
  overlay: (message, target) => setOverlay(message ? String(message) : null, target),
  stats: () => window.__HMR__ || {},
  router: () => Router,
}
```

`hooks.rs` calls three of the four: `bind` (every method definition), `stats`
(`live.rs` mirrors HMR telemetry into the `session` store, `protocol`'s refresh
button reads it), and `overlay` (the linter's panel). `router` is available to
JS-side debugging; Rust reaches the Router through `mjs::router()` instead,
which holds the class handed to `boot()` directly.

which is why every Rust method is written `|this, args|` — the scope object
first, the JS call's arguments as a real array second:

```rust
.method("inc", |this, args| {
    let step = js::arg_num(&args, 0, demo::COUNTER_STEP);
    js::set_num(&this, "count", js::get_num(&this, "count") + step);
    JsValue::UNDEFINED
})
```

`this` is m-js's **reactive proxy**, so writing through it schedules a coalesced
redraw — that's the whole reactivity model, no `M.redraw()` needed. Argument
helpers live in `js.rs`: `arg`, `arg_str`, `arg_num`, plus `get`/`get_str`/
`get_num`/`get_bool`/`set*` for objects and `call`/`call_global` for methods
(both report failures to the console instead of trapping the instance).

---

## 3 · Stores, and what survives a hot swap

`M.store()` keeps its bucket on `window`, so re-registering a definition after a
rebuild **merges new methods, adds new keys, and leaves existing data alone**.
That single fact is what makes Rust hot reload feel like real HMR.

The four stores (`app/src/store.rs`):

| store | shape | who writes it |
|---|---|---|
| `counter` | `count, clicks, lastOp` + `inc(n)`, `reset()` | the user — the value that must survive a swap |
| `meters` | `load, pctText, peak, phase` | `live.rs` at ~5 Hz |
| `session` | `api, builds, online, building, state, tone, lastFile, swaps, buildMs, buildLabel, buildTone, fetches, bootId, bootCount, error, lintCount, lintText, lintRules, rustTarget` | `api.rs` (fetch) + `live.rs` (from `window.__HMR__`) + `lib.rs` (boot) + `lint.rs` |
| `toasts` | `items[] {id,text,tone,at}`, `seq, ttl, max` + `push/dismiss/prune/clear` | anything, via `store::toast(text, tone)` |

Rules:

- **Anything a swap must survive lives in a store**, not in a Rust `static` or
  thread-local. New instance ⇒ new globals; Rust thread-locals are for caches
  and counters that are *allowed* to reset (frames, ticks, the gauge element).
- Mutating an array in place may not notify. `store.rs` rebuilds and re-`set`s
  the array (`toasts.push`/`dismiss`/`prune` all replace `items`).
- `live.rs` polls `window.__HMR__` through `hooks::stats()` and mirrors it into
  `session` — that's how a Rust panel displays JS-side HMR telemetry.

---

## 4 · The template linter (`kit/src/lint.rs`)

Balanced tags were half the problem. The other half is a *directive* that is
simply wrong, and m-js is silent about all of them:

| you write | m-js does |
|---|---|
| `x-text="$store.sesion.count"` | compiles, throws inside eval, returns `undefined` → blank |
| `x-text="count +"` | `new Function` throws → cached as `null` → blank, no console output |
| `x-component="uiButon"` | `console.warn('[m] unknown x-component')` at render → nothing |
| `x-for="items"` | `parseForExpression` → `null` → zero rows |
| `x-tex="label"` | not a directive → emitted as a plain DOM attribute → nothing |

So `boot()` runs `lint::run()` over the trees `Def::tree` recorded — structure,
never re-parsing markup — against seven rules:

`directive` · `syntax` (compiled the same way m-js compiles:
`new Function('$scope','$magics','with($magics){with($scope){return (EXPR)}}')`)
· `store` (every `$store.<name>` registered) · `component` (every
`x-component="<name>"` registered) · `for` (shaped `<item> in <list>`) · `key`
(no `:key` stranded on a static element) · `empty` (no empty directive value).

Findings go to `console.error`, into `$store.session.lint*` (Home paints them in
a red box), and into the same panel cargo's stderr uses:

```text
uiBadge · <span> x-tex [directive] no such directive `x-tex` — m-js will emit it as a plain attribute
pipelinePanel · <span> :value [syntax] SyntaxError: Unexpected token ')'
pipelinePanel · <span> :tone [store] `$store.sesion` is not registered (typo?) — m-js would render blank
```

`lint::self_test()` builds one tree carrying exactly one defect per rule and
asserts all seven fire, so **"template lint: clean" means the checker ran, not
that it was vacuous**. The count is in the Home footer and in
`__RUSTEX1__MOD.lintReport()`.

Caveats: it validates *syntax and names*, not semantics — a well-formed
expression reading a misspelled **property** (`$store.counter.coun`) still slips
through, and `x-if`/`x-show` conditions that are always false look identical to
a missing feature.

---

## 5 · Form field widgets (`kit/src/ui_form.rs`)

Seven components, one per kind of input, each shaped by a single metric: fewest
clicks and keystrokes to a valid value.

| widget | replaces | the shortcut it buys |
|---|---|---|
| `uiText` | `<input>` + label | `submit-on-enter`: type a line, press Enter, done — no click |
| `uiArea` | `<textarea>` | Enter stays a newline; ⌘/Ctrl+Enter submits |
| `uiNumber` | `<input type=number>` | − / + buttons |
| `uiDate` | `<input type=date>` | `today` `+1w` `+1mo` `clear` chips (opt out with `presets="false"`) |
| `uiChoice` | `<select>` | segmented control: 1 click, or ←/→ to walk |
| `uiChips` | comma-separated box | Enter/Space commits a token, Backspace pops, × removes |
| `uiToggle` | checkbox | one click on a row-sized target |

Host sugar keeps pages readable: `form::bar / text / area / number / date /
choice / chips / toggle / keys`, each taking the **field name** and the **model
expression** (`"$store.draft"`).

### Labelling and colouring a choice

`options` is the only required list. Three more are **positional against it** —
pass a comma list and index `i` belongs to option `i`:

| prop | gives option `i` | rendered as |
|---|---|---|
| `labels` | its word | `<span>` text (the default when no `icons`) |
| `icons` | a phosphor glyph name | `<i class="ph ph-*">` instead of the word; the word moves to the `ui-tooltip` |
| `tones` | a **badge tone name** | `t-<tone>` on the segment, so it carries that colour |

A tone is one of the five `ui-badge` tones — `ok | busy | warn | err | info`
(`--ok`, `--busy`, `--warn`, `--err`, `--info`). Reusing that vocabulary is the
point: an app maps a value to a tone **once** and both the form and the table
follow, so a status cannot read green in one and grey in the other. An untoned
segment keeps the old muted look (`--seg` falls back to `--muted`).

```rust
pub const STATUS_TONES: &[&str] = &["warn", "busy", "ok", "err"]; // idle running success fail

form::tinted(
    form::choice_icons("status", "status", "$store.draft", STATUSES, STATUS_ICONS),
    STATUS_TONES,
)
```

### Gestures `uiChoice` does not have

**No mouse wheel.** It had one (stepped the selection, wrapped at the ends,
throttled 70ms for trackpads) and it was removed: the pointer is over the control
because the user is *reading* it, and a gesture meant to move the page silently
rewrote a field instead — invisibly, until submit. Keyboard is the fast path: the
group takes `tabindex="0"` and `←/→ ↑/↓` step through `move()`, clamped at the
ends rather than wrapping.

If a hover gesture is ever wanted again it needs an affordance click has and
wheel does not — the user must have touched the control first.

### The value contract

A widget reads and writes `model[name]` on an object the host hands it:

```text
  host    <span x-component="uiText" name="summary" :model="$store.draft" label="Summary"></span>
  widget  reads  model[name]   (computed `value`)
          writes model[name]   (→ the reactive proxy schedules the redraw)
          plus a bubbling `field` event { name, value } for hosts that want it
```

Writing the store is primary because it is what actually drives the UI, and the
host needs no listener at all. The `field` event is published alongside it so the
documented m-js idiom — `@field="onField($event)"` on a container, for a dirty
flag or autosave — works too. `tasks1/web/probe.html` is the runnable check that
both still fire; keep it green.

Submit has two supported routes: pass the host's method as a prop
(`:on-submit="submit"`), or bind Enter on the host container and let it bubble
(`.on("keydown.enter", "submit()")`). `uiChips` uses `.stop` on Enter/Space so
committing a token cannot also submit the form.

### Three constraints that cost hours (all of them ours, not m-js's)

These were each misdiagnosed at first as "m-js dropped my handler". m-js was
right every time; a 20-line probe page settled it. They are recorded here so
nobody re-derives them:

* **The HTML parser lowercases attribute names.** `submitOnEnter=""` set from
  markup arrives as `submitonenter`, and `:onSubmit=` never matches `onSubmit`.
  Declare and set props in **kebab-case** (`submit-on-enter`, `:on-submit`).
* **`$el` inside a widget is its rendered root**, not the host `<span>`. Host
  attributes land on the *scope* as props, so `name` must be read from `this` —
  reading it off `$el` yields `undefined` and every event you dispatch carries
  `name: ""`, which the host then (correctly) ignores. Symptom: "my `@field`
  listener never fires", when it fired fine and the detail was empty.
* **`js::cb` already passes `this`.** A method closure is `(scope, callArgs)`, so
  the template's first argument is `callArgs[0]` — indexing from 1 silently reads
  `undefined`.

Mixed-case custom event names (`@Custom-Thing`) also cannot work: markup
lowercases the name, `dispatchEvent` does not, and event names are
case-sensitive. That is the platform, not m-js.

## 6 · Fast updates: one attribute per frame (`app/src/live.rs`)

m-js's diff is not the tool for 60 fps. The pattern the framework asks for, and
the one this repo uses:

- m-js renders the **structure** (the `<svg>`, track, labels) once from a
  template;
- the `requestAnimationFrame` loop writes **exactly one attribute** — the fill
  rect's `width` — through a cached element handle, and touches nothing else;
- anything coarser (the `%` label, toasts, panel content) is a **store write** at
  a few hertz, which is cheap for the diff.

Two invariants to keep if you add a fast widget:

1. **One DOM write per frame per widget.** If you need a second, it probably
   belongs in the 200 ms `housekeeping()` tick instead.
2. **Every loop must respect the boot token.** `lifecycle::on_frame` / `every` /
   `after` compare `window.__RUSTEX1__` before re-arming, so a superseded
   instance stops animating instead of fighting the new one for the same node.
   Never call `requestAnimationFrame` or `setTimeout` directly — always through
   `lifecycle`, and cache element handles keyed on `lifecycle::token()`.

`__RUSTEX1__MOD.debugState()` prints `boot=… frames=… ticks=… burst=…`; frames
advancing while your tab is visible is the proof the loop is owned by exactly one
instance.

---

## Adding things

### A widget

1. `app/src/components.rs` — add `fn my_widget()` building a `Def` with
   `.named("myWidget")`, declared props **with defaults**, and one `.tree(m(…))`;
   register it in `register()` and bump `COUNT`.
2. Style it in `web/styles.css` (class names are shared vocabulary between the
   templates and the stylesheet).
3. Add a card on the Storybook route so it's demonstrably reusable.
4. Save. Read the flash line, the build log row, and **`template lint 7/7`** in
   the footer. If a finding names your file, fix it before moving on.

Rules of thumb: props for host-provided data, state for widget-local data,
`$store.*` for anything shared or anything that must survive a swap. Templates
are `m()` trees — do not add a `template(r#"…"#)` string.

### A route

`app/src/pages.rs`:

```rust
mjs::router_register("/about", "rustex1 · about", |_| about());
```

then build the page as `Def::new().named("route /about").tree(m("div").class("shell").kids(vec![ Node::host("navBar", &[]), m("main").class("page")… ]))`.
Server-side SPA fallback already resolves a hard refresh on any extension-less
path, so path mode needs nothing from `server/`.

To add a nav entry, extend the `links` array in `nav_bar()` — hrefs are baked
from `Router.href()` at boot and the active state is computed by the widget's
`isActive` method, so it stays correct across navigation.

---

## Console toolbox

```js
window.__M__                       // the m-js namespace (store, component, redraw, parseTemplate…)
window.__M__.store('counter')      // read state a swap must survive
window.__Router__.uri              // current app path
window.__HMR__                     // connected, phase, file, swaps, pulse, lastBuildMs, error, lastMessage
__RUSTEX1__MOD.dumpTemplate('gaugeBar')   // the exact template string m-js got
__RUSTEX1__MOD.lintReport()                // linter findings, or "template lint: clean"
__RUSTEX1__MOD.debugState()                // boot token, frames, ticks, burst
__RUSTEX1__MOD.buildInfo()                  // rustc version + build stamp baked in by app/build.rs
```

## Known m-js behaviours we depend on (or work around)

- `@event` takes **any** event name. `parseDirective` recognises a leading `@`
  and splits the arg on `.`, with no list of allowed events behind it, so the
  HTML5 drag events (`@dragstart`, `@dragover.prevent`, `@drop.prevent`,
  `@dragend`) need nothing special — and `$event` is in scope for all of them,
  not just `click`. `makeHandler` forwards the live event into
  `evaluateAction`, which binds it as `$event` in the same `with($magics)` scope
  as everything else. Passing `$event` itself to a `Def` method is fine when you
  need `dataTransfer` or `clientY`; pass a *derived primitive*
  (`$event.shiftKey`) when you do not, which keeps the Rust off the event type.
- `x-bind:class` **adds** tokens to the static `class` and the value is rebuilt
  from the static attrs on every render — m-js never manages a token that the
  bound expression did not produce. Two consequences, both bitten:
  - an imperative `el.classList.add('is-dragging')` on an `x-for` row is *not*
    cleaned up by a redraw, so whatever sets it has to clear it;
  - clearing through the element the gesture **started** on is wrong once the
    list has re-rendered and reordered underneath it — that node may be
    detached. Clear by querying the document for the marked class instead.
- `draggable` needs a payload before a drag will start: Firefox fires nothing at
  all unless `dataTransfer.setData` is called in `dragstart`. Set it there, read
  it back in `drop`, and do not keep the dragged path in store state — it is one
  string that already lives on the event.

- `parseTemplate` uses `<template>.innerHTML` → malformed markup is **repaired,
  never rejected**. (§1)
- Static template siblings are keyed **positionally** (`_${i}`); `keyToSid` only
  applies to `x-for` rows, so `:key` on a static element is a no-op. (§4)
- Re-registering a component updates the registry but live instances keep their
  state *and their old template string* (`widgetTemplate` prefers
  `scope.template`). Hence unmount-on-swap. (§"boot sequence")
- `evaluate()` catches every thrown expression and returns `undefined`, and
  caches a failed compile as `null` → silent blanks. (§4)
- `M.store(name, def)` merges functions, adds missing keys, preserves data;
  `init()` runs once, only on first creation. (§3)
- Any write through a reactive proxy schedules one coalesced redraw **on a
  frame** — so a hidden/background tab shows stale DOM until it is painted
  again. Not a bug; it fooled us once during development.
- There is no `$router` magic — routes are reached via a scope method
  (`@click="go($event)"` → `Router.link(event)`), and `Router.href(uri)` gives
  the prefixed URL.
- Redraws with unchanged state write nothing to the DOM, which is why a 200 ms
  telemetry tick writing a handful of store fields is free.

## What we deliberately did **not** do

- No fork, patch, or vendored copy of m-js — everything here is additive, on top
  of the published CDN bundle and its documented two-message hot-reload protocol
  (`{type:'connected'}` then `{type:'change', path}`). The extra message types
  (`build`, `restart`, `bootId`) are additions a **stock** m-js client safely
  ignores.
- No build tooling for the client: no bundler, no npm install, no transpiler.
  `cargo` is the only build step, and the dev server runs it.

*Framework reference: <https://mikesmullin.github.io/m-js/> · project overview:
[../README.md](../README.md).*
