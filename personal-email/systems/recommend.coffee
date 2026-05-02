import '../microagents/01-recommend-action.coffee'
import { _G } from '../../lib/globals.coffee'

export recommendSystem = ->
  entities = _G.World.Entity__find (e) -> e.recall? and not e.recommendation?
  for entity in entities
    _G.currentEntityId = entity.id
    { content, recall } = entity
    result = await _G.recommendActionMicroagent content.body, recall.journalContext
    await _G.Entity.patch entity, 'recommendation',
      journal_id: result.journal_id
      ref: result.ref
      operations: result.operations
      rationale: result.rationale
      confidence: Number(result.confidence ? 0)
      label: "(#{result.ref}) #{result.operations}. #{result.rationale}."
