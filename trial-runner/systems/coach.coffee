import '../microagents/11-coach-row.coffee'
import '../../personal-email/microagents/09-build-journal-entry.coffee'
import { _G } from '../../lib/globals.coffee'

###
 Coach system — per-entity backprop coaching, runs inside the batch loop.

 Respects _G.pipelineWidth.  Only processes entities whose seance phase is
 already complete (or not applicable):
   - Passed entities:  seance never runs → coach immediately.
   - Failed, no retrospective context:  seance skipped → coach immediately.
   - Failed, has retrospective context:  must wait until e.seance? is set.

 For each entity:
   PASS → carry forward trial_rationale unchanged as backprop_rationale.
   FAIL → call coach-row LLM for feedback + new backprop_rationale
          (using seance-verified or best-effort rationale when available).

 After determining backprop_rationale, writes a corrected journal entry to
 _G.MEMO_DB using backprop_rationale as the instruction signal.  This lets
 subsequent batches' recallSystem surface the corrected guidance before those
 emails are processed — the core benefit of running coach inside the loop.

 Persists on entity:
   _trial.backprop_rationale  – for the generation chain (next trial run)
   coached                    – { feedback, seance_note } for reportSystem
###
export coachSystem = ->
  entities = (_G.World.Entity__find (e) ->
    e.trial_result? and not e.coached? and
    (e.trial_result.passed or e.seance? or not e.retrospective?.stage_6_context?)
  )[0..._G.pipelineWidth]

  for entity in entities
    _G.currentEntityId = entity.id
    { content, trial_result, seance, _trial } = entity

    verifiedRationale   = seance?.verified_backprop_rationale or null
    bestEffortRationale = seance?.best_effort_rationale or null
    seanceVerified      = seance?.verification_passed is true
    status              = if trial_result.passed then '✅ PASS' else '❌ FAIL'

    if trial_result.passed
      backpropRationale = trial_result.trial_rationale
      feedback          = 'CONTINUE: Rationale correctly guided the agent. No changes needed.'
      seanceNote        = ''
    else
      coachRow = await _G.traceStep '🎓', "Coaching #{status} #{entity.id} (got: #{trial_result.given}, expected: #{trial_result.correct})", ->
        _G.coachRowMicroagent(
          entity.id,
          trial_result.given,
          trial_result.correct,
          trial_result.trial_rationale,
          trial_result.original_rationale,
          if verifiedRationale or bestEffortRationale
            "Seance #{if seanceVerified then 'VERIFIED' else 'best-effort'} rationale: #{verifiedRationale or bestEffortRationale}"
          else ''
        )

      # Priority: seance-verified > coach-row > seance best-effort
      backpropRationale = verifiedRationale or coachRow.backprop_rationale or bestEffortRationale or ''
      feedback          = coachRow.feedback or ''
      seanceNote        = if seanceVerified then ' ✓seance' else if seance? then ' ⚠seance' else ''

    # Persist backprop_rationale for generation chain
    await _G.Entity.patch entity, '_trial',
      { ...(entity._trial or {}), backprop_rationale: backpropRationale }

    # Store coaching output for reportSystem (avoids re-running LLM there)
    await _G.Entity.patch entity, 'coached',
      feedback:    feedback
      seance_note: seanceNote

    # Write corrected journal entry so subsequent batches recall the improved signal
    journalEntry = await _G.buildJournalEntryMicroagent(
      content.body,
      backpropRationale,     # corrected instruction signal
      trial_result.correct,  # confirmed correct answer
    )
    await _G.saveJournalEntry _G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry

    _G.log 'trial.coach.done', { id: entity.id, passed: trial_result.passed, seanceNote }
