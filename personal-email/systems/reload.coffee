import { _G } from '../../lib/globals.coffee'
import { refreshSystem } from './refresh.coffee'

MICROAGENT_FILES = [
  '00-fingerprint-email.coffee'
  '01-recommend-action.coffee'
  '02-contains-question.coffee'
  '04-answer-question-from-email.coffee'
  '05-summarize-email.coffee'
  '06-presentation-rule-relevance.coffee'
  '07-execute-memo-instruction.coffee'
  '08-execute-instruction.coffee'
  '09-build-journal-entry.coffee'
  '10-build-presentation-entry.coffee'
  '14-journal-entry-relevance-filter.coffee'
  '15-consolidate-journal-group.coffee'
]

export reloadSystem = (microagentDir) ->
  entities = _G.World.Entity__find (e) -> e.operator?.command is 'reload'
  for entity in entities
    _G.currentEntityId = entity.id
    trace = _G.Entity.traceStart entity, '🔄', 'Reloading microagent modules from disk'
    t = Date.now()
    for f in MICROAGENT_FILES
      await import("#{microagentDir}/#{f}?t=#{t}")
    entity = await trace.traceEnd()
    await _G.Entity.log entity, 'Microagents reloaded. Re-evaluating email...'
  await refreshSystem()
