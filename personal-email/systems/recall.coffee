import '../microagents/06-presentation-rule-relevance.coffee'
import { _G } from '../../lib/globals.coffee'

export recallSystem = ->
  entities = _G.World.Entity__find (e) -> e.fingerprint? and not e.recall?
  for entity in entities
    _G.currentEntityId = entity.id
    { content, envelope, fingerprint } = entity

    { context: journalContext } = await _G.hybridJournalRecallLib(
      _G.spawn, _G.MEMO_DB, content.body, fingerprint, { senderEmail: envelope.senderEmail }
    )
    presentationMatches = await _G.recallJournal _G.spawn, _G.PRESENTATION_MEMO_DB, content.body
    presentationCandidate = await _G.extractPresentationPreferences content.body, presentationMatches
    usePresentationPreferences = if presentationCandidate?.has_formatting_instructions
      await _G.isRelevant content.body, presentationCandidate.applies_if
    else
      false

    await _G.Entity.patch entity, 'recall',
      journalContext: journalContext
      presentationCandidate: presentationCandidate ? null
      usePresentationPreferences: usePresentationPreferences
