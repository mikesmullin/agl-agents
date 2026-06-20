# email-trainer-2 back-end server
# Bun HTTP + WebSocket + poll-based file watcher
#
# Differences from email-trainer (v1):
#   - Configuration is hard-coded (no config.yaml / config.yaml.example).
#   - No local db/ directory.
#   - Adds GET /api/entities/:id/raw to expose the original email HTML body
#     (for secure, sandboxed rendering in the detail view).

import { load as yamlLoad, loadAll as yamlLoadAll, dump as yamlDump } from 'js-yaml'
import { readFileSync, writeFileSync, readdirSync, statSync, existsSync, unlinkSync, rmSync } from 'fs'
import { resolve, extname } from 'path'

# ---------------------------------------------------------------------------
# Paths + hard-coded config
# ---------------------------------------------------------------------------
__dir = new URL('.', import.meta.url).pathname.replace /\/$/, ''

PORT         = 4001
ENTITIES_DIR = resolve __dir, '../personal-email/db/entities'
POLL_MS      = 3000
PUBLIC_DIR   = resolve __dir, 'public'

# Read folder destinations from personal-email/config.yaml (google_email.labels)
PERSONAL_EMAIL_CONFIG_PATH = resolve __dir, '../personal-email/config.yaml'
DESTINATIONS = do ->
  try
    raw = readFileSync PERSONAL_EMAIL_CONFIG_PATH, 'utf8'
    parsed = yamlLoad raw
    labels = parsed?.google_email?.labels
    if labels and typeof labels is 'object' and not Array.isArray labels
      Object.keys labels
    else
      []
  catch
    []

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
    entity = yamlLoad text
    entity.id = String(id) if entity  # YAML may parse leading-zero IDs as integers
    entity
  catch
    null

# Strip origin.raw (full HTML body) from API/WS responses — too large to push
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
  entity = yamlLoad text
  entity.id = String(id) if entity  # YAML may parse leading-zero IDs as integers
  updated = deepMerge entity, patch
  writeFileSync path, yamlDump(updated, { indent: 2 }), 'utf8'
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

# Extract the original HTML (or plain) email body from origin.raw.
# origin.raw is a YAML-serialized dump of the source Gmail message; parse it and
# pluck body.content. Returns { contentType, content } or null.
extractRawBody = (entity) ->
  raw = entity?.origin?.raw
  return null unless raw
  try
    parsed = yamlLoad raw
    body = parsed?.body
    if body?.content?
      return { contentType: (body.contentType or 'text'), content: String(body.content) }
  catch
  # Fallback: treat the whole raw blob as plain text
  { contentType: 'text', content: String(raw) }

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

  # GET /api/entities/:id/raw — original email body for sandboxed rendering
  rawMatch = pathname.match /^\/api\/entities\/([^/]+)\/raw$/
  if method is 'GET' and rawMatch
    id = rawMatch[1]
    unless /^[a-zA-Z0-9_-]+$/.test id
      return new Response JSON.stringify(error: 'invalid id'),
        status: 400, headers: { 'Content-Type': 'application/json' }
    entity = readEntity id
    unless entity
      return new Response JSON.stringify(error: 'not found'),
        status: 404, headers: { 'Content-Type': 'application/json' }
    body = extractRawBody entity
    return new Response JSON.stringify({
      id: id
      contentType: body?.contentType ? null
      content:     body?.content     ? null
      text:        entity.content?.body ? null
    }), headers: { 'Content-Type': 'application/json' }

  # GET /api/archive — list all _archive/*.yaml entities with journal cross-reference
  if method is 'GET' and pathname is '/api/archive'
    archiveDir  = resolve ENTITIES_DIR, '../_archive'
    journalPath = resolve ENTITIES_DIR, '../journal.yaml'
    try
      # Load journal once for cross-referencing
      journalMap = {}
      if existsSync journalPath
        journalDocs = []
        yamlLoadAll readFileSync(journalPath, 'utf8'), (doc) -> journalDocs.push doc if doc
        for doc in journalDocs
          journalMap[String doc.id] = doc if doc?.id?

      files = readdirSync archiveDir
      entries = files
        .filter (f) -> f.endsWith '.yaml'
        .map (f) ->
          try
            text   = readFileSync resolve(archiveDir, f), 'utf8'
            entity = yamlLoad text
            return null unless entity
            entity.id = String f.replace /\.yaml$/, ''
            slim = trimEntity entity
            out =
              id:             slim.id
              envelope:
                from: slim.envelope?.from   ? null
              apply:
                applied_at: slim.apply?.applied_at ? null
              summary:
                headline:    slim.summary?.headline    ? null
                description: slim.summary?.description ? null
              recommendation:
                journal_id:  slim.recommendation?.journal_id  ? null
                operations:  slim.recommendation?.operations  ? null
                confidence:  slim.recommendation?.confidence  ? null
                rationale:   slim.recommendation?.rationale   ? null
              operator_input:
                instruction:      slim.operator_input?.instruction      ? null
                _parsed_operation: slim.operator_input?._parsed_operation ? null
              execution:
                instruction: slim.execution?.instruction ? null
            jid = slim.recommendation?.journal_id
            if jid?
              jentry = journalMap[String jid]
              if jentry?.metadata
                out.journal_meta =
                  confirmed_count:    jentry.metadata.confirmed_count    ? null
                  last_confirmed_ts:  jentry.metadata.last_confirmed_ts  ? null
            out
          catch
            null
        .filter Boolean
      return new Response JSON.stringify(entries),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify([]),
        headers: { 'Content-Type': 'application/json' }

  # DELETE /api/archive/:id — permanently delete an archived entity file
  archiveDeleteMatch = pathname.match /^\/api\/archive\/([^/]+)$/
  if method is 'DELETE' and archiveDeleteMatch
    id = archiveDeleteMatch[1]
    unless /^[a-zA-Z0-9_-]+$/.test id
      return new Response JSON.stringify(error: 'invalid id'),
        status: 400, headers: { 'Content-Type': 'application/json' }
    archiveDir = resolve ENTITIES_DIR, '../_archive'
    filePath = resolve archiveDir, "#{id}.yaml"
    try
      unlinkSync filePath
      return new Response JSON.stringify(ok: true),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify(error: e.message),
        status: 404, headers: { 'Content-Type': 'application/json' }

  # GET /api/trials — list trial run report-cards
  if method is 'GET' and pathname is '/api/trials'
    trialDir = resolve ENTITIES_DIR, '../_archive/trial'
    try
      dirs = readdirSync trialDir
      entries = dirs
        .filter (d) ->
          try statSync(resolve(trialDir, d)).isDirectory()
          catch then false
        .map (d) ->
          try
            folderSt = statSync resolve(trialDir, d)
            base =
              id:          d
              date:        folderSt.mtime.toISOString()
              passing:     null
              total:       null
              score:       null
              grade:       null
              duration_ms: null
            rcPath = resolve trialDir, d, 'report-card.md'
            return base unless existsSync rcPath
            text  = readFileSync rcPath, 'utf8'
            rcSt  = statSync rcPath
            base.date = (rcSt.birthtimeMs > 0 and rcSt.birthtimeMs isnt rcSt.ctimeMs) and rcSt.birthtime.toISOString() or rcSt.mtime.toISOString()
            rcMs     = rcSt.birthtimeMs > 0 and rcSt.birthtimeMs isnt rcSt.ctimeMs and rcSt.birthtimeMs or rcSt.mtimeMs
            folderMs = folderSt.birthtimeMs > 0 and folderSt.birthtimeMs isnt folderSt.ctimeMs and folderSt.birthtimeMs or folderSt.mtimeMs
            base.duration_ms = if rcMs > folderMs then rcMs - folderMs else null
            scoreMatch = text.match /\*\*Score:\*\*\s+\((\d+)\/(\d+)\)\s+(\d+)%\s+=\s+Grade\s+([A-F][+-]?)/
            return base unless scoreMatch
            base.passing = parseInt scoreMatch[1], 10
            base.total   = parseInt scoreMatch[2], 10
            base.score   = parseInt scoreMatch[3], 10
            base.grade   = scoreMatch[4]
            base
          catch
            null
        .filter Boolean
      return new Response JSON.stringify(entries),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify([]),
        headers: { 'Content-Type': 'application/json' }

  # DELETE /api/trials/:id — permanently delete a trial folder
  trialDeleteMatch = pathname.match /^\/api\/trials\/([^/]+)$/
  if method is 'DELETE' and trialDeleteMatch
    id = trialDeleteMatch[1]
    unless /^[a-zA-Z0-9_-]+$/.test id
      return new Response JSON.stringify(error: 'invalid id'),
        status: 400, headers: { 'Content-Type': 'application/json' }
    trialDir = resolve ENTITIES_DIR, '../_archive/trial'
    folderPath = resolve trialDir, id
    try
      rmSync folderPath, { recursive: true, force: true }
      return new Response JSON.stringify(ok: true),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify(error: e.message),
        status: 404, headers: { 'Content-Type': 'application/json' }

  # POST /api/trials/:id/promote — run bun trial-runner/agent.coffee --promote <id>
  trialPromoteMatch = pathname.match /^\/api\/trials\/([^/]+)\/promote$/
  if method is 'POST' and trialPromoteMatch
    id = trialPromoteMatch[1]
    unless /^[a-zA-Z0-9_-]+$/.test id
      return new Response JSON.stringify(error: 'invalid id'),
        status: 400, headers: { 'Content-Type': 'application/json' }
    agentRoot = resolve __dir, '..'
    try
      result = Bun.spawnSync ['bun', 'trial-runner/agent.coffee', '--promote', id],
        cwd: agentRoot
        stdout: 'pipe'
        stderr: 'pipe'
      stdout = result.stdout?.toString?() ? ''
      stderr = result.stderr?.toString?() ? ''
      if result.exitCode is 0
        return new Response JSON.stringify(ok: true, output: stdout),
          headers: { 'Content-Type': 'application/json' }
      else
        return new Response JSON.stringify(error: stderr or stdout, exitCode: result.exitCode),
          status: 500, headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify(error: e.message),
        status: 500, headers: { 'Content-Type': 'application/json' }

  # ---------------------------------------------------------------------------
  # Trial-run control API
  # ---------------------------------------------------------------------------
  LOCK_PATH  = resolve ENTITIES_DIR, '../_archive/trial/running.lock'
  AGENT_ROOT = resolve __dir, '..'

  _isProcessRunning = (pid) ->
    try
      process.kill pid, 0
      true
    catch
      false

  _readLock = ->
    return null unless existsSync LOCK_PATH
    try
      data = JSON.parse readFileSync LOCK_PATH, 'utf8'
      alive = _isProcessRunning data.pid
      { ...data, alive }
    catch
      null

  # GET /api/trial-run/status — returns lock state + latest progress.yaml
  if method is 'GET' and pathname is '/api/trial-run/status'
    lock = _readLock()
    if lock and not lock.alive
      try unlinkSync LOCK_PATH catch
      lock = null
    progressData = null
    TRIAL_BASE_DIR = resolve ENTITIES_DIR, '../_archive/trial'
    tryRunId = lock?.run_id
    if not tryRunId
      try
        entries = readdirSync(TRIAL_BASE_DIR).filter (name) ->
          try statSync(resolve(TRIAL_BASE_DIR, name)).isDirectory() catch then false
        entries.sort()
        tryRunId = entries[entries.length - 1] if entries.length > 0
      catch
    if tryRunId
      pPath = resolve TRIAL_BASE_DIR, tryRunId, 'progress.yaml'
      try
        progressData = yamlLoad readFileSync pPath, 'utf8'
      catch
    return new Response JSON.stringify({ running: lock?, lock, progress: progressData }),
      headers: { 'Content-Type': 'application/json' }

  # POST /api/trial-run/start — spawn trial-runner as a background process
  if method is 'POST' and pathname is '/api/trial-run/start'
    lock = _readLock()
    if lock?.alive
      return new Response JSON.stringify(error: "Trial run #{lock.run_id} already running (pid #{lock.pid})"),
        status: 409, headers: { 'Content-Type': 'application/json' }
    body = {}
    try body = await req.json() catch
    flags = ['trial-runner/agent.coffee']
    if body.fromId and /^[a-zA-Z0-9]+$/.test body.fromId
      flags.push '--from-id', body.fromId
    try
      proc = Bun.spawn ['bun', ...flags],
        cwd: AGENT_ROOT
        stdout: 'pipe'
        stderr: 'pipe'
      proc.stdout?.pipeTo?(new WritableStream({ write: -> })).catch(->) ? proc.stdout?.cancel?()
      proc.stderr?.pipeTo?(new WritableStream({ write: -> })).catch(->) ? proc.stderr?.cancel?()
      try
        writeFileSync LOCK_PATH,
          JSON.stringify({ pid: proc.pid, run_id: null, started_at: new Date().toISOString() })
      catch
      return new Response JSON.stringify(ok: true, pid: proc.pid),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify(error: e.message),
        status: 500, headers: { 'Content-Type': 'application/json' }

  # POST /api/trial-run/stop — send SIGTERM to the running trial
  if method is 'POST' and pathname is '/api/trial-run/stop'
    lock = _readLock()
    unless lock?.alive
      try unlinkSync LOCK_PATH catch
      return new Response JSON.stringify(ok: true, note: 'no running trial found'),
        headers: { 'Content-Type': 'application/json' }
    try
      process.kill lock.pid, 'SIGTERM'
      try unlinkSync LOCK_PATH catch
      return new Response JSON.stringify(ok: true, stopped_pid: lock.pid),
        headers: { 'Content-Type': 'application/json' }
    catch e
      return new Response JSON.stringify(error: e.message),
        status: 500, headers: { 'Content-Type': 'application/json' }

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

    if url.pathname is '/ws'
      ok = server.upgrade req
      return undefined if ok
      return new Response 'WebSocket upgrade failed', { status: 400 }

    if url.pathname.startsWith '/api/'
      return await handleAPI req, url

    serveStatic url.pathname

  websocket:
    open: (ws) ->
      wsClients.add ws
      entities = loadAllEntities().map trimEntity
      ws.send JSON.stringify { type: 'init', entities }

    close: (ws) ->
      wsClients.delete ws

    message: (ws, msg) ->
      try
        data = JSON.parse msg
        console.log 'ws message:', data.type

# ---------------------------------------------------------------------------
# Startup banner
# ---------------------------------------------------------------------------
console.log ''
console.log '  ✉️  email-trainer-2'
console.log "  🌐  http://localhost:#{PORT}"
console.log "  📁  #{ENTITIES_DIR}"
console.log ''
