import '../microagents/13-seance-coach.coffee'
import '../../personal-email/microagents/01-recommend-action.coffee'
import { _G } from '../../lib/globals.coffee'

MAX_SEANCE_ITERATIONS = 10

###
 Seance system — tight coaching loop for failed entities.

 For each entity that failed the operator gate:
   1. Show the seance-coach the email, correct answer, current trial_rationale,
      and the running history of failed attempts.
   2. Coach proposes a candidate backprop_rationale.
   3. Run a fresh recommendActionMicroagent call with the candidate as journal
      context (using the model that ran during the trial).
   4. If the agent now chooses the correct answer → verified; loop exits.
   5. Otherwise → record the failed attempt, continue to next iteration.
   6. After MAX_SEANCE_ITERATIONS the best-effort last candidate is stored.

 The coach conversation is stateful across iterations: each call receives the
 accumulated messages from the prior coach turn so it builds on its own reasoning
 without repeating context.

 Output stored in entity.seance:
   verified_backprop_rationale  – rationale that produced the correct answer (or null)
   verification_passed          – true if we found a working rationale
   attempts                     – array of { rationale, got } for failed iterations
###
export seanceSystem = ->
  entities = _G.World.Entity__find (e) ->
    e.trial_result? and not e.trial_result.passed and
    e.retrospective?.stage_6_context? and not e.seance?

  for entity in entities
    _G.currentEntityId = entity.id
    { content, trial_result, retrospective, _trial } = entity

    model          = retrospective.stage_6_model or _G.MODEL
    correctAnswer  = trial_result.correct
    trialRationale = _trial?.trial_rationale or ''

    # Temporarily point the recommend microagent at the trial model
    savedModel = _G.MODEL
    _G.MODEL = model

    previousAttempts = []
    verifiedRationale = null
    coachMessages = null  # accumulates across iterations for stateful coach context
    iteration = 0

    while iteration < MAX_SEANCE_ITERATIONS and not verifiedRationale?
      iteration++
      _G.log 'trial.seance.iteration', { id: entity.id, iteration, max: MAX_SEANCE_ITERATIONS }

      # Coach proposes a candidate rationale
      { result: coachResult, ctx: newCoachMessages } = await _G.seanceCoachMicroagent(
        content.body,
        correctAnswer,
        trialRationale,
        previousAttempts,
        coachMessages,
      )
      coachMessages = newCoachMessages  # carry context into next iteration

      candidate = String(coachResult?.backprop_rationale or '').trim()
      unless candidate then continue

      # Verification: run fresh recommend with the candidate as journal context
      testResult = await _G.recommendActionMicroagent content.body, candidate

      got = String(testResult?.operations or '').trim()
      passed = got.toLowerCase() is correctAnswer.toLowerCase()

      _G.log 'trial.seance.attempt',
        { id: entity.id, iteration, candidate, got, correctAnswer, passed }

      if passed
        verifiedRationale = candidate
      else
        previousAttempts.push { rationale: candidate, got }

    # Restore model
    _G.MODEL = savedModel

    lastCandidate = if previousAttempts.length
      previousAttempts[previousAttempts.length - 1].rationale
    else trialRationale

    await _G.Entity.patch entity, 'seance',
      verified_backprop_rationale: verifiedRationale
      verification_passed: verifiedRationale?
      attempts: previousAttempts
      best_effort_rationale: if verifiedRationale then null else lastCandidate

    status = if verifiedRationale
      "✅ verified in #{previousAttempts.length + 1} attempt(s)"
    else
      "⚠️ unverified after #{MAX_SEANCE_ITERATIONS} attempts"
    _G.log 'trial.seance.done', { id: entity.id, status, verifiedRationale }
    await _G.Entity.log entity, "Seance: #{status}"
