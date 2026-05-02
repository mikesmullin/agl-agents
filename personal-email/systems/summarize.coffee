import '../microagents/05-summarize-email.coffee'
import { _G } from '../../lib/globals.coffee'

export summarizeSystem = ->
  entities = _G.World.Entity__find (e) -> e.recall? and not e.summary?
  for entity in entities
    _G.currentEntityId = entity.id
    { content, recall } = entity
    presentationText = _G.optionalText recall.usePresentationPreferences, recall.presentationCandidate?.formatting_instructions
    summary = await _G.summarizeEmailMicroagent content.body, presentationText
    await _G.Entity.patch entity, 'summary', summary
