/**
 * rustex1 entry point — shipped by the kit, served at `/app.mjs`.
 *
 * 1 · m-js comes from the cloud-hosted CDN bundle — one import, zero install.
 * 2 · the UI comes from Rust compiled to wasm; *which* wasm comes from
 *     `GET /api/config`, so this file never mentions the app's crate name.
 * 3 · `window.__M_BOOT__` is the hook the hot client calls after a rebuild, so
 *     a Rust edit re-registers components and re-diffs instead of reloading.
 *
 * An app may shadow this file by putting its own `web/app.mjs` in front of the
 * kit's static root — no copy step, and `git submodule update` refreshes it.
 */

import M, { Router } from 'https://mikesmullin.github.io/m-js/dist/m.min.js'
import { config, overlay as setOverlay } from './hot.mjs'

/**
 * A wasm-exported closure has no notion of a JS receiver, and wasm-bindgen has
 * no variadic closures — so arguments travel as a real Array and every Rust
 * method is written `|this, args| { … }`. `kit/src/hooks.rs` calls these.
 */
const hooks = {
  bind: (fn) =>
    function bound(...args) {
      return fn(this, args)
    },
  overlay: (message, target) => setOverlay(message ? String(message) : null, target),
  stats: () => window.__HMR__ || {},
  router: () => Router,
  // children talk up: the form widgets in kit/src/ui_form.rs dispatch through here
  dispatch: (el, type, detail) =>
    el?.dispatchEvent?.(new CustomEvent(type, { detail, bubbles: true, composed: true })) ?? false,
}

function mountPoint() {
  return document.getElementById('app') || document.body
}

function setSplash(on, message) {
  let splash = document.getElementById('splash')
  if (!splash && on) {
    splash = document.createElement('div')
    splash.id = 'splash'
    splash.className = 'splash'
    splash.innerHTML =
      '<div class="splash-ring"></div><p>compiling rust → wasm…</p>' +
      '<p class="muted tiny">the dev server builds the frontend before it answers the first request</p>'
    document.body.appendChild(splash)
  }
  if (splash) {
    splash.hidden = !on
    if (message) splash.querySelector('p').textContent = message
  }
}

setSplash(true)

let attempts = 0

function fail(err) {
  console.error('[rustex1] boot failed', err)
  // A restart or a rebuild landing mid-download truncates the .wasm response.
  // That is transient — try again before blaming the code.
  if (attempts++ < 3 && !document.body?.dataset?.rustReady) {
    setTimeout(() => boot(Date.now()).then(() => (attempts = 0)).catch(fail), 600)
    return
  }
  setSplash(true, 'wasm boot failed — see the panel below')
  let box = document.querySelector('.app-error')
  if (!box) {
    box = document.createElement('pre')
    box.className = 'app-error'
    document.body.appendChild(box)
  }
  box.hidden = false
  box.textContent = `boot() threw:\n\n${err?.stack || err}`
  setOverlay(String(err?.stack || err), 'wasm boot')
}

let booting = Promise.resolve()

export async function boot(t = Date.now()) {
  const { pkgJs, pkgWasm } = await config()
  const bust = String(t)
  // Cache-bust both the glue module and the binary: without this the browser
  // hands back the previous build and the "hot reload" silently does nothing.
  const mod = await import(`${pkgJs}?t=${bust}`)
  await mod.default({ module_or_path: new URL(`${pkgWasm}?t=${bust}`, location.href) })
  await mod.boot(M, Router, hooks, { bootId: bust })
  attempts = 0
  setSplash(false)
  window.__RUSTEX1__MOD = mod
  document.body.dataset.rustReady = '1'
  document.body.dataset.buildInfo =
    typeof mod.buildInfo === 'function' ? mod.buildInfo() : ''
  return mod
}

/** Serialize boots — a burst of file events must not interleave two mounts. */
export function enqueueBoot(t) {
  booting = booting.then(() => boot(t)).catch(fail)
  return booting
}

window.__M_BOOT__ = (t) => enqueueBoot(t)
window.__M__ = M
window.__Router__ = Router

await boot()
