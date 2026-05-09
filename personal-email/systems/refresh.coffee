import { _G } from '../../lib/globals.coffee'

export resetSystem = ->
  entities = _G.World.Entity__find (e) ->
    e.operator_input?.instruction is 'reset'
  for entity in entities
    _G.currentEntityId = entity.id
    await _G.traceStep '🔄', 'Resetting entity', ->
      _G.Entity.save { id: entity.id, origin: { raw: entity.origin?.raw } }
