import { _G } from '../../lib/globals.coffee'

###
 Trial-mode operator gate.

 Compares recommendation.operations (what the LLM chose this trial run)
 against entity._trial.correct_answer (what the human operator originally
 accepted, i.e. the reward signal).

 Pass  → given === correct (case-insensitive, trimmed)
 Fail  → given !== correct

 Result is stored in entity.trial_result for use by the report system.
###
export operatorSystem = ->
  entities = _G.World.Entity__find (e) -> e.recommendation? and not e.trial_result?
  for entity in entities
    _G.currentEntityId = entity.id
    { recommendation, _trial } = entity

    given   = String(recommendation?.operations or '').trim()
    correct = String(_trial?.correct_answer or '').trim()
    passed  = given.toLowerCase() is correct.toLowerCase()

    await _G.Entity.patch entity, 'trial_result',
      given: given
      correct: correct
      passed: passed
      trial_rationale: _trial?.trial_rationale or ''
      original_rationale: _trial?.original_rationale or ''

    status = if passed then '✅ PASS' else '❌ FAIL'
    _G.log 'trial.operator.result', { id: entity.id, given, correct, passed }
    await _G.Entity.log entity, "#{status} given=#{JSON.stringify given} correct=#{JSON.stringify correct}"
