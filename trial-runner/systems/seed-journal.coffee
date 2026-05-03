import '../../personal-email/microagents/09-build-journal-entry.coffee'
import '../../lib/memo.coffee'
import { _G } from '../../lib/globals.coffee'

###
 Seed a fresh trial journal from each entity's current trial_rationale.

 Runs after loadSystem and fingerprintSystem so content.body is available,
 and before recallSystem so the journal exists when hybrid search runs.

 For each entity:
   - Calls buildJournalEntryMicroagent with the email body, the current
     trial_rationale as the operator instruction, and correct_answer as the
     execution outcome.
   - Saves the resulting structured entry (with keywords, sender_email
     metadata, factors, rule, etc.) to the per-trial journal at _G.MEMO_DB.

 The trial journal is written fresh each run — no cumulative state across
 generations.  The generation chain lives in entity._trial.backprop_rationale
 → next run's trial_rationale → next run's journal entries.
###
export seedJournalSystem = ->
  entities = _G.World.Entity__find (e) -> e.content? and e.fingerprint? and e._trial?

  await Promise.all entities.map (entity) ->
    _G.currentEntityId = entity.id
    { content, _trial } = entity

    rationale    = String(_trial?.trial_rationale   or '').trim() or 'No rationale.'
    correctAnswer = String(_trial?.correct_answer    or '').trim() or 'unknown'

    journalEntry = await _G.buildJournalEntryMicroagent(
      content.body,
      rationale,      # instruction — the coaching rationale being tested this generation
      correctAnswer,  # executionOutcome — what action was actually taken
    )

    await _G.saveJournalEntry _G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry

    _G.log 'trial.seed-journal.entry', { id: entity.id, correctAnswer, rationale }
