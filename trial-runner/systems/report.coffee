import '../microagents/11-coach-row.coffee'
import '../microagents/12-coach-summary.coffee'
import { writeFile } from 'fs/promises'
import { resolve } from 'path'
import { _G } from '../../lib/globals.coffee'

gradeFromPct = (pct) ->
  if pct >= 90 then 'A'
  else if pct >= 80 then 'B'
  else if pct >= 70 then 'C'
  else if pct >= 60 then 'D'
  else 'F'

###
 Report system — runs at the end of a trial.

 For FAILED entities:
   - If seance produced a verified_backprop_rationale, use it directly.
     Coach-row then focuses on writing feedback text only.
   - If seance ran but didn't verify (all iterations exhausted), use
     seance.best_effort_rationale as the backprop_rationale input to coach-row.
   - If no seance data, coach-row handles everything as before.

 For PASSED entities:
   - Coach-row generates both feedback and backprop_rationale as before.

 Writes report-card.md and persists backprop_rationale on each entity's _trial
 component so the generation chain can seed the next run.
###
export reportSystem = (trialDir, runId) ->
  entities = _G.World.Entity__find (e) -> e.trial_result?

  rows = []
  for entity in entities
    _G.currentEntityId = entity.id
    { trial_result, seance } = entity

    # Determine backprop_rationale source for failed entities
    verifiedRationale   = seance?.verified_backprop_rationale or null
    bestEffortRationale = seance?.best_effort_rationale or null
    seanceVerified      = seance?.verification_passed is true

    if trial_result.passed
      # PASS: skip coach LLM — carry forward the rationale that just worked
      rows.push
        entityId: entity.id
        passed: true
        given: trial_result.given
        correct: trial_result.correct
        trialRationale: trial_result.trial_rationale
        originalRationale: trial_result.original_rationale
        feedback: 'CONTINUE: Rationale correctly guided the agent. No changes needed.'
        backpropRationale: trial_result.trial_rationale
        seanceNote: ''

      await _G.Entity.patch entity, '_trial',
        { ...(entity._trial or {}), backprop_rationale: trial_result.trial_rationale }

      continue

    # FAIL: call coach-row for feedback + new backprop_rationale
    coachRow = await _G.coachRowMicroagent(
      entity.id,
      trial_result.given,
      trial_result.correct,
      trial_result.trial_rationale,
      trial_result.original_rationale,
      # Pass seance context when available
      if verifiedRationale or bestEffortRationale
        "Seance #{if seanceVerified then 'VERIFIED' else 'best-effort'} rationale: #{verifiedRationale or bestEffortRationale}"
      else ''
    )

    # backprop_rationale priority for failures:
    #   1. Seance-verified (proven to work at least once)
    #   2. Coach-row output (best judgment without verification)
    backpropRationale = verifiedRationale or coachRow.backprop_rationale or bestEffortRationale or ''

    seanceNote = if seanceVerified then ' ✓seance' else if seance? then ' ⚠seance' else ''

    rows.push
      entityId: entity.id
      passed: false
      given: trial_result.given
      correct: trial_result.correct
      trialRationale: trial_result.trial_rationale
      originalRationale: trial_result.original_rationale
      feedback: coachRow.feedback or ''
      backpropRationale: backpropRationale
      seanceNote: seanceNote

    # Persist backprop_rationale for generation chain (FAIL path only; PASS is persisted above)
    await _G.Entity.patch entity, '_trial',
      { ...(entity._trial or {}), backprop_rationale: backpropRationale }

  # Overall score
  passed = rows.filter((r) -> r.passed).length
  total  = rows.length
  pct    = if total > 0 then Math.round(passed / total * 100) else 0
  grade  = gradeFromPct pct

  summary = await _G.coachSummaryMicroagent rows

  # Build markdown report card
  tableHeader = '| Entity | Result | Given Answer | Correct Answer | Trial Rationale | Original Rationale | Feedback | Backprop Rationale |'
  tableSep    = '|--------|--------|--------------|----------------|-----------------|---------------------|----------|-------------------|'
  tableRows   = rows.map (r) ->
    status = if r.passed then '✅ PASS' else "❌ FAIL#{r.seanceNote}"
    entityLink = "[#{r.entityId}](entities/#{r.entityId}.yaml)"
    "| #{entityLink} | #{status} | #{r.given} | #{r.correct} | #{r.trialRationale} | #{r.originalRationale} | #{r.feedback} | #{r.backpropRationale} |"

  scoreLabel = "(#{passed}/#{total}) #{pct}% = Grade #{grade}"

  md = """
    # Trial Run #{runId} — Report Card

    > Agent: [personal-email](../../../../../personal-email/README.md)

    ## Results

    #{tableHeader}
    #{tableSep}
    #{tableRows.join '\n'}

    ## Summary

    **Score:** #{scoreLabel}

    **Encouragement:** #{summary.encouragement or ''}
    """

  reportPath = resolve trialDir, 'report-card.md'
  await writeFile reportPath, md.trimStart(), 'utf8'
  _G.log 'trial.report.written', { reportPath, score: scoreLabel }

  { rows, passed, total, pct, grade, scoreLabel }
