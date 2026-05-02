import { _G } from '../../lib/globals.coffee'

export planSystem = ->
  entities = _G.World.Entity__find (e) -> e.journal? and not e.plan?
  return unless entities.length

  result = await _G.traceStep '📋', 'Planning mutations', -> _G.planEmailTransactionLib()
  planText = (result.stdout or result.stderr or '').trim()

  for entity in entities
    _G.currentEntityId = entity.id
    entity = await _G.Entity.log entity, "plan:\n#{planText}"
    await _G.Entity.patch entity, 'plan',
      success: result.code is 0
      text: planText
      planned_at: new Date().toISOString()
