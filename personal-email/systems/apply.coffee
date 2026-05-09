import { _G } from '../../lib/globals.coffee'

export applySystem = ->
  # Write apply gate for entities with a plan not yet approved
  pending = (_G.World.Entity__find (e) -> e.plan? and not e.apply?)[0..._G.pipelineWidth]
  for entity in pending
    _G.currentEntityId = entity.id
    unless entity.apply?.approved?
      await _G.Entity.patch entity, 'apply', { approved: null }

  # Execute apply when operator has set approved: true
  entities = (_G.World.Entity__find (e) -> e.apply?.approved is true and not e.apply?.applied_at?)[0..._G.pipelineWidth]
  return unless entities.length

  for entity in entities
    _G.currentEntityId = entity.id
    result = await _G.traceStep '🚀', 'Applying mutations', -> _G.applyEmailTransactionLib(entity.id)
    applyText = (result.stdout or result.stderr or '').trim()

    entity = await _G.Entity.log entity, "apply: #{applyText}"
    updated = { ...entity, apply: { ...entity.apply, success: result.code is 0, output: applyText, applied_at: new Date().toISOString() } }
    await _G.Entity.save updated
