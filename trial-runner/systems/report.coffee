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
 Report system — runs once after all batches complete.

 Reads coaching data already computed by coachSystem (which ran per-batch
 inside the main loop).  No per-entity LLM calls here — just scoring,
 the summary LLM call, and writing report-card.md.
###
export reportSystem = (trialDir, runId) ->
  entities = _G.World.Entity__find (e) -> e.coached?

  rows = entities.map (entity) ->
    { trial_result, coached, _trial } = entity
    entityId: entity.id
    passed:   trial_result.passed
    given:    trial_result.given
    correct:  trial_result.correct
    trialRationale:    trial_result.trial_rationale
    originalRationale: trial_result.original_rationale
    feedback:          coached.feedback
    backpropRationale: entity._trial?.backprop_rationale or ''
    seanceNote:        coached.seance_note

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
