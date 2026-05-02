import { _G } from '../../lib/globals.coffee'

export refreshSystem = ->
  entities = _G.World.Entity__find (e) -> e.operator?.command is 'refresh' or e.operator?.command is 'reload'
  for entity in entities
    _G.currentEntityId = entity.id
    await _G.traceStep '🔄', 'Clearing components', ->
      _G.Entity.clearComponents entity, ['operator_input', 'fingerprint', 'recall', 'summary', 'recommendation', 'operator', 'execution', 'journal']
