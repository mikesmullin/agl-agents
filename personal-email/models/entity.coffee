import { readFile, writeFile, mkdir, rm, readdir, rename } from 'fs/promises'
import { resolve } from 'path'
import { YAML } from 'bun'
import { _G } from '../../lib/globals.coffee'

ENTITY_DIR = resolve process.cwd(), 'personal-email/db/entities'
ARCHIVE_DIR = resolve process.cwd(), 'personal-email/db/_archive'

_G.Entity = class Entity
  @init: ->
    await mkdir ENTITY_DIR, { recursive: true }
    # Load all existing entity YAMLs from disk into World on startup
    files = await readdir ENTITY_DIR
    for file in files
      if file.endsWith '.yaml'
        id = file.replace /\.yaml$/, ''
        await @load id

  @_path: (id) ->
    resolve ENTITY_DIR, "#{id}.yaml"

  @load: (id) ->
    try
      text = await readFile @_path(id), 'utf8'
      entity = YAML.parse(text) ? { id }
    catch
      entity = { id }
    _G.World.set entity
    entity

  @save: (entity) ->
    await writeFile @_path(entity.id), YAML.stringify(entity, null, 2), 'utf8'
    _G.World.set entity
    entity

  @patch: (entity, componentName, data) ->
    updated = { ...entity, [componentName]: data }
    await @save updated
    _G.log "entity.patch.#{componentName}", { id: entity.id, ...data }
    updated

  @archive: (id) ->
    await mkdir ARCHIVE_DIR, { recursive: true }
    try
      await rename @_path(id), resolve(ARCHIVE_DIR, "#{id}.yaml")
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
    _G.World.Entity__find((e) -> e.id is entity.id)[0] ? entity

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
      fresh = _G.World.Entity__find((e) -> e.id is entityId)[0] ? entity
      updated = { ...fresh, traces: [...(fresh.traces or []), { emoji, label, ms }] }
      await _G.Entity.save updated
      updated