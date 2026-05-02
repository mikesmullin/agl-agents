import '../microagents/08-execute-instruction.coffee'
import { _G } from '../../lib/globals.coffee'

export executeSystem = ->
  entities = _G.World.Entity__find (e) ->
    e.operator? and not e.execution? and e.operator.command not in ['skip', 'quit']
  for entity in entities
    _G.currentEntityId = entity.id
    { operator } = entity
    trace = _G.Entity.traceStart entity, '⚙️', 'Executing instruction'
    result = await _G.executeInstructionMicroagent entity.id, operator.instruction
    entity = await trace.traceEnd()
    entity = await _G.Entity.log entity, "#{if result.success then '✅' else '❌'} #{result.summary}"
    await _G.Entity.patch entity, 'execution',
      success: result.success
      summary: result.summary
      instruction: operator.instruction
