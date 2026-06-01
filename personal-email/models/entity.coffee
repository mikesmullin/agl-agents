import { readFile, writeFile, mkdir, readdir, rename, stat } from 'fs/promises'
import { resolve, basename } from 'path'
import { load as yamlLoad, dump as yamlDump } from 'js-yaml'
import chokidar from 'chokidar'
import { _G } from '../../lib/globals.coffee'

_entityDir = -> _G.ENTITY_DIR or resolve process.cwd(), 'personal-email/db/entities'
_archiveDir = -> _G.ARCHIVE_DIR or resolve process.cwd(), 'personal-email/db/_archive'

_G.Entity = class Entity
  @init: ->
    await mkdir _entityDir(), { recursive: true }
    # Load all existing entity YAMLs from disk into World on startup
    files = await readdir _entityDir()
    for file in files
      if file.endsWith '.yaml'
        id = file.replace /\.yaml$/, ''
        await @load id
    # Watch for operator edits — only re-parse the one file that changed
    chokidar.watch _entityDir(),
      ignoreInitial: true
      awaitWriteFinish: { stabilityThreshold: 120, pollInterval: 50 }
    .on 'change', (p) =>
      id = basename p, '.yaml'
      await @load id if p.endsWith '.yaml'
    .on 'add', (p) =>
      id = basename p, '.yaml'
      await @load id if p.endsWith '.yaml'
    .on 'unlink', (p) =>
      id = basename p, '.yaml'
      _G.World.remove id if p.endsWith '.yaml'

  @_path: (id) ->
    resolve _entityDir(), "#{id}.yaml"

  # Internal: unconditionally read + parse from disk, update World.
  @_loadFromDisk: (id) ->
    path = @_path id
    try
      { mtimeMs } = await stat path
      process.stdout.write '.'
      text = await readFile path, 'utf8'
      entity = yamlLoad(text) ? { id }
      entity._mtime = mtimeMs
    catch
      entity = { id }
    entity.id = String(id)  # filename is canonical; YAML may misparse IDs like 0e6836 as numbers
    _G.World.set entity
    entity

  # Public: skip disk read if mtime unchanged (for explicit reload calls).
  @load: (id) ->
    path = @_path id
    try
      { mtimeMs } = await stat path
      cached = _G.World.get String(id)
      return cached if cached?._mtime is mtimeMs
    catch
      # File doesn't exist yet — register a stub so loadSystem can pick it up
      existing = _G.World.get String(id)
      unless existing
        stub = { id: String(id) }
        _G.World.set stub
        return stub
      return existing
    await @_loadFromDisk id

  @save: (entity) ->
    { _mtime, toWrite... } = entity
    path = @_path entity.id
    await writeFile path, yamlDump(toWrite, { indent: 2 }), 'utf8'
    # Capture the new mtime so the chokidar 'change' event (and any explicit
    # load() calls) can see this write came from us and skip reparsing.
    try
      { mtimeMs } = await stat path
      entity = { ...toWrite, _mtime: mtimeMs }
    catch
      entity = toWrite
    _G.World.set entity
    entity

  @patch: (entity, componentName, data) ->
    updated = { ...entity, [componentName]: data }
    await @save updated
    _G.log "entity.patch.#{componentName}", { id: entity.id, ...data }
    updated

  @archive: (id) ->
    await mkdir _archiveDir(), { recursive: true }
    try
      await rename @_path(id), resolve(_archiveDir(), "#{id}.yaml")
    catch # file may not exist
    _G.World.remove id

  # @delete: (id) ->
  #   try
  #     await rm @_path(id)
  #   catch # file may not exist
  #   _G.World.remove id

  @clearComponents: (entity, componentNames) ->
    updated = { ...entity }
    delete updated[name] for name in componentNames
    await @save updated
    updated

  @_fresh: (entity) ->
    _G.World.get(entity.id) ? entity

  @log: (entity, message) ->
    fresh = @_fresh entity
    entry = "[#{new Date().toISOString()}] #{message}"
    updated = { ...fresh, log: [...(fresh.log or []), entry] }
    await @save updated
    updated

  @traceStart: (entity, emoji, label) ->
    entityId = entity.id
    stdoutTrace = _G.traceStart emoji, label
    started = Date.now()
    traceEnd: ->
      stdoutTrace.traceEnd()
      ms = Date.now() - started
      fresh = _G.World.get(entityId) ? entity
      updated = { ...fresh, traces: [...(fresh.traces or []), { emoji, label, ms }] }
      await _G.Entity.save updated
      updated