/**
 * rustex1 HMR client — shipped by the kit, served at `/hot.mjs`.
 *
 * Speaks m-js's hot-reload protocol — one socket at `/__m_hmr`, `{type:'connected'}`
 * then `{type:'change', path}` — so the rules the framework documents still hold:
 *
 *   .css            → clone + cache-bust the <link>, no redraw
 *   .js / .mjs      → re-import with a cache-bust, then window.__M_BOOT__(t)
 *   .html           → full reload (the document shell changed)
 *
 * plus three messages a rustex1 dev server adds, which a stock m-js client
 * simply ignores:
 *
 *   {type:'build',   phase:'start'|'ok'|'error', target, ms, error}
 *   {type:'restart'}                              → the backend process is being replaced
 *   {type:'connected', bootId}                    → a bootId change forces one full reload
 *
 * Nothing here knows the app's name: the wasm artifact URLs come from
 * `GET /api/config`. State the Rust side reads back lives on `window.__HMR__`.
 */

const HMR_PROTO = location.protocol === 'https:' ? 'wss' : 'ws'

/** Filled in by config(); defaults are what every rustex1 app uses. */
const cfg = {
  app: 'app',
  pkgJs: '/pkg/app.js',
  pkgWasm: '/pkg/app_bg.wasm',
  hmrPath: '/__m_hmr',
}

let configPromise = null

/** `GET /api/config`, once. app.mjs and the .rs fallback both use it. */
export function config() {
  if (!configPromise) {
    configPromise = fetch('/api/config', { cache: 'no-store' })
      .then((r) => (r.ok ? r.json() : {}))
      .then((json) => Object.assign(cfg, json || {}))
      .catch(() => cfg)
  }
  return configPromise
}

const stats = (window.__HMR__ ||= {
  connected: false,
  phase: 'idle',
  file: '',
  swaps: 0,
  pulse: 0,
  lastBuildMs: 0,
  error: '',
  lastMessage: '',
  bootId: '',
  events: [],
})

const root = document.documentElement

// ---------------------------------------------------------------------------
// the pieces of chrome this client owns — created on demand, so an app's
// index.html only has to contain <div id="app">
// ---------------------------------------------------------------------------

function ensure(tag, id, className, parent = document.body) {
  if (id) {
    const found = document.getElementById(id)
    if (found) return found
  }
  const el = document.createElement(tag)
  if (id) el.id = id
  if (className) el.className = className
  parent.appendChild(el)
  return el
}

// ---------------------------------------------------------------------------
// flash line (a 1s strip so rebuilds are visible while demoing)
// ---------------------------------------------------------------------------

let flashTimer = null
function flash(text, tone = 'ok') {
  const el = ensure('div', 'hmr-flash', 'hmr-flash')
  el.textContent = text
  el.dataset.tone = tone
  el.hidden = false
  clearTimeout(flashTimer)
  flashTimer = setTimeout(() => (el.hidden = true), 1400)
}

// ---------------------------------------------------------------------------
// diagnostic overlay (pushed, not polled — the server already has the text, and
// the Rust side can call this through the hooks bridge)
//
// The template linter owns this panel. Rust compile errors deliberately do NOT:
// cargo's stderr is already in the terminal running the build, which is where a
// developer reads it, and a fixed panel parked over the app was worse than
// useless for a failure that is somebody else's console output. See the `build`
// handler — the failure still lands in `__HMR__.error` and the console.
// ---------------------------------------------------------------------------

function overlay(message, target) {
  const box = ensure('div', 'hmr-overlay')
  if (!message) {
    box.hidden = true
    return
  }
  if (!box.querySelector('.overlay-head')) {
    const head = document.createElement('div')
    head.className = 'overlay-head'
    head.innerHTML =
      '<i class="ph ph-hard-drives"></i><b id="overlay-title">rust</b><span class="grow"></span>' +
      '<button type="button" id="overlay-close" class="btn tiny">dismiss</button>'
    box.appendChild(head)
    const pre = document.createElement('pre')
    pre.id = 'overlay-body'
    box.appendChild(pre)
  }
  const head = box.querySelector('#overlay-title')
  // The panel is the linter's. Anything else that pushes into it is titled by
  // its target — never claimed to be a build failure, which is no longer routed
  // here at all.
  head.textContent =
    target === 'template lint'
      ? 'template lint — fix the directives below'
      : target
        ? String(target)
        : 'diagnostics'
  box.querySelector('#overlay-body').textContent = message
  box.hidden = false
}

document.addEventListener('click', (e) => {
  if (e.target?.id === 'overlay-close') overlay(null)
})

// ---------------------------------------------------------------------------
// the m-js side of things
// ---------------------------------------------------------------------------

function reloadCss(href) {
  const clean = href.split('?')[0]
  const links = [...document.querySelectorAll('link[rel="stylesheet"]')]
  const matches = links.filter((link) => {
    const url = new URL(link.href, location.href)
    return url.pathname === clean || url.pathname.endsWith(clean) || clean.endsWith(url.pathname)
  })

  for (const link of matches) {
    const next = link.cloneNode()
    next.href = clean + '?t=' + Date.now()
    next.onload = () => link.remove()
    link.parentNode.insertBefore(next, link.nextSibling)
  }
  if (!matches.length) {
    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.href = clean + '?t=' + Date.now()
    document.head.appendChild(link)
  }
  console.debug('[m.hmr] css', clean)
}

async function reloadJs(path) {
  const bust = Date.now()
  stats.swaps += 1
  stats.pulse += 1
  const url = path.startsWith('/') ? path : '/' + path
  try {
    // Preload so __M_BOOT__ resolves the already-fresh module instance.
    await import(url + '?t=' + bust)
  } catch (err) {
    console.warn('[m.hmr] preload failed', err)
  }
  if (typeof window.__M_BOOT__ === 'function') {
    await window.__M_BOOT__(bust)
    console.debug('[m.hmr] boot ok', bust)
  } else {
    const entry = document.querySelector('script[data-hmr-entry]')
    await import((entry?.getAttribute('src') || '/app.mjs') + '?t=' + bust)
  }
}

// ---------------------------------------------------------------------------
// coalescer: one rebuild touches /pkg/*.js and /pkg/*.wasm — that is one boot
// ---------------------------------------------------------------------------

let pending = { css: new Set(), js: new Set(), html: false }
let flushTimer = null
let queue = Promise.resolve()

function schedule() {
  if (flushTimer) return
  flushTimer = setTimeout(() => {
    flushTimer = null
    const batch = pending
    pending = { css: new Set(), js: new Set(), html: false }

    if (batch.html) {
      flash('document changed → reload', 'warn')
      location.reload()
      return
    }
    batch.css.forEach(reloadCss)
    if (batch.js.size) {
      const path = [...batch.js][0]
      root.dataset.hmr = 'updating'
      root.dataset.hmrFile = path
      queue = queue
        .then(() => reloadJs(path))
        .then(() => {
          window.dispatchEvent(new CustomEvent('m:hmr', { detail: { path } }))
        })
        .catch((err) => {
          console.error('[m.hmr] boot failed → full reload', err)
          location.reload()
        })
        .finally(() => {
          root.dataset.hmr = stats.connected ? 'connected' : 'disconnected'
        })
    }
  }, 40)
}

function classify(path) {
  stats.file = path
  if (/\.css$/i.test(path)) pending.css.add(path)
  else if (/\.(html|htm)$/i.test(path)) pending.html = true
  else if (/\.(js|mjs|ts)$/i.test(path)) pending.js.add(path)
  else if (/\.wasm$/i.test(path)) pending.js.add(path.replace(/_bg\.wasm$/, '.js'))
  else if (/\.rs$|Cargo\.toml$/i.test(path)) pending.js.add(cfg.pkgJs)
  else return false
  schedule()
  return true
}

// ---------------------------------------------------------------------------
// socket
// ---------------------------------------------------------------------------

let socket = null
let backoff = 500

function connect() {
  try {
    socket = new WebSocket(`${HMR_PROTO}://${location.host}${cfg.hmrPath}`)
  } catch (err) {
    console.warn('[m.hmr] websocket unavailable', err)
    return
  }

  socket.addEventListener('open', () => {
    backoff = 500
    console.debug('[m.hmr] connected')
  })

  socket.addEventListener('message', (ev) => {
    let msg
    try {
      msg = JSON.parse(ev.data)
    } catch {
      return
    }
    stats.lastMessage = ev.data

    switch (msg.type) {
      case 'connected': {
        stats.connected = true
        root.dataset.hmr = 'connected'
        if (stats.bootId && msg.bootId && stats.bootId !== msg.bootId) {
          flash('rust server restarted → reload', 'warn')
          location.reload()
          return
        }
        stats.bootId = msg.bootId || ''
        if (!String(stats.phase).startsWith('rebuilding')) stats.phase = 'live'
        console.debug('[m.hmr] server hello', msg)
        break
      }

      case 'change': {
        if (msg.path) classify(msg.path)
        break
      }

      case 'build': {
        if (msg.phase === 'start') {
          stats.phase = `rebuilding ${msg.target || ''}`.trim()
          stats.error = ''
          overlay(null)
          flash(`cargo ${msg.target} · ${(msg.trigger || []).join(' ')}`, 'busy')
        } else if (msg.phase === 'ok') {
          stats.phase = 'live'
          stats.lastBuildMs = msg.ms || 0
          stats.error = ''
          overlay(null)
          flash(`${msg.target} rebuilt in ${msg.ms}ms`, 'ok')
          // Tell the app its telemetry moved, if it cares.
          window.__RUSTEX1__MOD?.pulse?.()
        } else if (msg.phase === 'error') {
          stats.phase = 'failed'
          stats.error = msg.error || 'build failed'
          // No overlay on purpose: the compile errors are already in the shell
          // running cargo. One short flash, and the full text kept where a
          // program can still read it — the console and `__HMR__.error`.
          console.error(`[m.hmr] ${msg.target} build failed\n${stats.error}`)
          flash(`${msg.target} build failed`, 'err')
        }
        window.dispatchEvent(new CustomEvent('m:hmr', { detail: { build: msg } }))
        break
      }

      case 'restart': {
        stats.phase = 'restarting server'
        flash('restarting the rust server…', 'warn')
        break
      }

      default:
        break
    }
  })

  socket.addEventListener('close', () => {
    stats.connected = false
    root.dataset.hmr = 'disconnected'
    setTimeout(connect, backoff)
    backoff = Math.min(backoff * 2, 4000)
  })

  socket.addEventListener('error', () => socket?.close())
}

if (!window.__M_HMR_STARTED__) {
  window.__M_HMR_STARTED__ = true
  config().then(connect) // know the artifact URLs before the first message lands
}

export { connect, overlay, stats, flash }
export default { connect, overlay, stats, flash, config }
