# email-trainer back-end server
# Bun HTTP + WebSocket + poll-based file watcher

import { YAML } from 'bun'
import { readFileSync, writeFileSync, readdirSync, statSync, existsSync, mkdirSync } from 'fs'
import { resolve, extname } from 'path'

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
__dir = new URL('.', import.meta.url).pathname.replace /\/$/, ''

DEFAULT_CONFIG =
  port: 4000
  entities_dir: '../personal-email/db/entities'
  poll_interval_ms: 3000

configPath = resolve __dir, 'config.yaml'
config = do ->
  return DEFAULT_CONFIG unless existsSync configPath
  try
    YAML.parse readFileSync configPath, 'utf8'
  catch e
    console.warn "config.yaml parse error: #{e.message}"
    DEFAULT_CONFIG

PORT         = config.port             or DEFAULT_CONFIG.port
ENTITIES_DIR = resolve __dir, (config.entities_dir or DEFAULT_CONFIG.entities_dir)
POLL_MS      = config.poll_interval_ms or DEFAULT_CONFIG.poll_interval_ms

# Read folder destinations from personal-email/config.yaml (google_email.labels)
# Falls back to an empty array; the front-end will still show the hard-coded preset.
PERSONAL_EMAIL_CONFIG_PATH = resolve __dir, '../personal-email/config.yaml'
DESTINATIONS = do ->
  try
    raw = readFileSync PERSONAL_EMAIL_CONFIG_PATH, 'utf8'
    parsed = YAML.parse raw
    labels = parsed?.google_email?.labels
    if labels and typeof labels is 'object' and not Array.isArray labels
      Object.keys labels
    else
      []
  catch
    []
PUBLIC_DIR   = resolve __dir, 'public'
DB_DIR       = resolve __dir, 'db'

mkdirSync DB_DIR, { recursive: true }

# ---------------------------------------------------------------------------
# MIME types
# ---------------------------------------------------------------------------
MIME =
  '.html': 'text/html; charset=utf-8'
  '.css':  'text/css; charset=utf-8'
  '.js':   'application/javascript; charset=utf-8'
  '.json': 'application/json'
  '.ico':  'image/x-icon'
  '.png':  'image/png'
  '.svg':  'image/svg+xml'
  '.woff2': 'font/woff2'
  '.woff':  'font/woff'

# ---------------------------------------------------------------------------
# WebSocket clients
# ---------------------------------------------------------------------------
wsClients = new Set()

broadcast = (msg) ->
  json = JSON.stringify msg
  wsClients.forEach (ws) ->
    try ws.send json

# ---------------------------------------------------------------------------
# Entity helpers
# ---------------------------------------------------------------------------
entityPath = (id) -> resolve ENTITIES_DIR, "#{id}.yaml"

readEntity = (id) ->
  try
    text = readFileSync entityPath(id), 'utf8'
    YAML.parse text
  catch
    null

# Strip origin.raw (full HTML body) from API responses — too large for WS
trimEntity = (entity) ->
  return entity unless entity
  trimmed = { ...entity }
  if trimmed.origin?
    trimmed.origin = { ...trimmed.origin }
    delete trimmed.origin.raw
  trimmed

deepMerge = (target, source) ->
  result = { ...target }
  for own key, val of source
    if val isnt null and typeof val is 'object' and not Array.isArray(val) and
       result[key]? and typeof result[key] is 'object' and not Array.isArray(result[key])
      result[key] = deepMerge result[key], val
    else
      result[key] = val
  result

patchEntityFile = (id, patch) ->
  path = entityPath id
  throw new Error "Entity not found: #{id}" unless existsSync path
  text = readFileSync path, 'utf8'
  entity = YAML.parse text
  updated = deepMerge entity, patch
  writeFileSync path, YAML.stringify(updated), 'utf8'
  updated

listEntityIds = ->
  try
    readdirSync ENTITIES_DIR
      .filter (f) -> f.endsWith '.yaml'
      .map    (f) -> f.replace /\.yaml$/, ''
  catch
    []

loadAllEntities = ->
  listEntityIds()
    .map (id) -> readEntity id
    .filter Boolean

# ---------------------------------------------------------------------------
# Poll-based file watcher
# id → { mtime: number, size: number }
# ---------------------------------------------------------------------------
fileSnap = new Map()

# Seed initial snapshot without broadcasting
do ->
  for id in listEntityIds()
    try
      s = statSync entityPath id
      fileSnap.set id, { mtime: s.mtimeMs, size: s.size }

pollEntities = ->
  try
    files = readdirSync ENTITIES_DIR
    currentIds = new Set()

    for file in files
      continue unless file.endsWith '.yaml'
      id = file.replace /\.yaml$/, ''
      currentIds.add id
      fullPath = resolve ENTITIES_DIR, file
      try
        s = statSync fullPath
        prev = fileSnap.get id
        if not prev
          entity = readEntity id
          if entity
            fileSnap.set id, { mtime: s.mtimeMs, size: s.size }
            broadcast { type: 'entity:new', entity: trimEntity entity }
        else if s.mtimeMs isnt prev.mtime or s.size isnt prev.size
          entity = readEntity id
          if entity
            fileSnap.set id, { mtime: s.mtimeMs, size: s.size }
            broadcast { type: 'entity:modified', entity: trimEntity entity }

    # Detect deletions
    for id from fileSnap.keys()
      unless currentIds.has id
        fileSnap.delete id
        broadcast { type: 'entity:deleted', id }

  catch e
    console.error "poll error: #{e.message}"

setInterval pollEntities, POLL_MS

# ---------------------------------------------------------------------------
# Static file server
# ---------------------------------------------------------------------------
serveStatic = (pathname) ->
  clean = pathname.split('?')[0]
  filePath =
    if clean is '/' or not clean
      resolve PUBLIC_DIR, 'index.html'
    else
      resolve PUBLIC_DIR, clean.replace /^\//, ''

  mimeType = MIME[extname filePath] or 'application/octet-stream'
  try
    data = readFileSync filePath
    new Response data, headers: { 'Content-Type': mimeType }
  catch
    # SPA fallback — serve index.html for any non-found path
    try
      html = readFileSync resolve PUBLIC_DIR, 'index.html'
      new Response html, headers: { 'Content-Type': 'text/html; charset=utf-8' }
    catch
      new Response 'Not Found', { status: 404 }

# ---------------------------------------------------------------------------
# API handler
# ---------------------------------------------------------------------------
handleAPI = (req, url) ->
  { pathname } = url
  { method }   = req

  # GET /api/entities
  if method is 'GET' and pathname is '/api/entities'
    entities = loadAllEntities().map trimEntity
    return new Response JSON.stringify(entities),
      headers: { 'Content-Type': 'application/json' }

  # GET /api/config
  if method is 'GET' and pathname is '/api/config'
    return new Response JSON.stringify({
      port: PORT
      poll_interval_ms: POLL_MS
      destinations: DESTINATIONS
    }), headers: { 'Content-Type': 'application/json' }

  # PATCH /api/entities/:id
  match = pathname.match /^\/api\/entities\/([^/]+)$/
  if method is 'PATCH' and match
    id = match[1]
    try
      body = await req.json()
      updated = patchEntityFile id, body
      return new Response JSON.stringify(trimEntity updated),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify({ error: e.message }),
        status: 400
        headers: { 'Content-Type': 'application/json' }

  new Response 'Not Found', { status: 404 }

# ---------------------------------------------------------------------------
# Bun HTTP + WebSocket server
# ---------------------------------------------------------------------------
server = Bun.serve
  port: PORT

  fetch: (req, server) ->
    url = new URL req.url

    # WebSocket upgrade
    if url.pathname is '/ws'
      ok = server.upgrade req
      return undefined if ok
      return new Response 'WebSocket upgrade failed', { status: 400 }

    # API
    if url.pathname.startsWith '/api/'
      return await handleAPI req, url

    # Static files
    serveStatic url.pathname

  websocket:
    open: (ws) ->
      wsClients.add ws
      entities = loadAllEntities().map trimEntity
      ws.send JSON.stringify { type: 'init', entities }

    close: (ws) ->
      wsClients.delete ws

    message: (ws, msg) ->
      # All mutations go through REST; WS is read-only push from server
      try
        data = JSON.parse msg
        console.log 'ws message:', data.type

# ---------------------------------------------------------------------------
# Startup banner
# ---------------------------------------------------------------------------
console.log ''
console.log '  ✉️  email-trainer'
console.log "  🌐  http://localhost:#{PORT}"
console.log "  📁  #{ENTITIES_DIR}"
console.log ''
