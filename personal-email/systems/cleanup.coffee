import { _G } from '../../lib/globals.coffee'

CLEANUP_DELAY_MS = 60_000

export cleanSystem = ->
  now = Date.now()
  entities = _G.World.Entity__find (e) ->
    e.apply?.success is true and e.apply?.applied_at? and
    (now - new Date(e.apply.applied_at).getTime()) >= CLEANUP_DELAY_MS
  for entity in entities
    _G.currentEntityId = entity.id
    await _G.traceStep '🗑️', 'Archiving completed entity', ->
      _G.Entity.archive entity.id
