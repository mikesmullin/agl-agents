import '../microagents/09-build-journal-entry.coffee'
import '../microagents/10-build-presentation-entry.coffee'
import { _G } from '../../lib/globals.coffee'

export journalSystem = ->
  entities = _G.World.Entity__find (e) -> e.execution? and not e.journal?
  for entity in entities
    _G.currentEntityId = entity.id
    { content, operator, execution, recommendation } = entity

    if operator.command is 'proceed' and recommendation.journal_id > 0
      await _G.reinforceJournalEntryLib _G.spawn, _G.DB_DIR, _G.MEMO_DB, recommendation.journal_id

    journalEntry = await _G.buildJournalEntryMicroagent(
      content.body, operator.instructionOrRecommendation, execution.summary
    )
    await _G.saveJournalEntry _G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry

    presentationEntry = await _G.hasFormattingInstructions operator.instruction, content.body
    if presentationEntry?.has_formatting_instructions
      await _G.savePresentationEntry _G.spawn, _G.DB_DIR, _G.PRESENTATION_MEMO_DB, presentationEntry

    await _G.Entity.patch entity, 'journal', journalEntry
