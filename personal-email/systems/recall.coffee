import '../microagents/06-presentation-rule-relevance.coffee'
import { _G } from '../../lib/globals.coffee'

# Writes recall.journalContext via hybrid journal search.
# Gate: fingerprint present, journalContext not yet set.
export recallJournalSystem = ->
  entities = (_G.World.Entity__find (e) -> e.fingerprint? and not e.recall?.journalContext?)[0..._G.pipelineWidth]
  for entity in entities
    _G.currentEntityId = entity.id
    { content, envelope, fingerprint } = entity

    { context: journalContext } = await _G.hybridJournalRecallLib(
      _G.spawn, _G.MEMO_DB, content.body, fingerprint, { senderEmail: envelope.senderEmail }
    )
    await _G.Entity.patch entity, 'recall',
      { ...(entity.recall or {}), journalContext }

# Writes recall.{presentationCandidate, usePresentationPreferences} via presentation memo search.
# Gate: fingerprint present, usePresentationPreferences not yet set.
# Only run by personal-email; trial-runner preserves these fields from the archive entity.
export recallPresentationSystem = ->
  entities = (_G.World.Entity__find (e) -> e.fingerprint? and not e.recall?.usePresentationPreferences?)[0..._G.pipelineWidth]
  for entity in entities
    _G.currentEntityId = entity.id
    { content } = entity

    presentationMatches = await _G.recallJournal _G.spawn, _G.PRESENTATION_MEMO_DB, content.body
    presentationCandidate = await _G.extractPresentationPreferences content.body, presentationMatches
    usePresentationPreferences = if presentationCandidate?.has_formatting_instructions
      await _G.isRelevant content.body, presentationCandidate.applies_if
    else
      false

    await _G.Entity.patch entity, 'recall',
      { ...(entity.recall or {}), presentationCandidate: presentationCandidate ? null, usePresentationPreferences }

# Combined convenience export used by personal-email/agent.coffee.
export recallSystem = ->
  await recallJournalSystem()
  await recallPresentationSystem()
