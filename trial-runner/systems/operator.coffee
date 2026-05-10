import { readFile } from 'fs/promises'
import { resolve } from 'path'
import { YAML } from 'bun'
import { _G } from '../../lib/globals.coffee'

# ---------------------------------------------------------------------------
# Load valid Gmail label names from personal-email config.
# These define the complete set of accepted "move to <folder>" destinations.
# ---------------------------------------------------------------------------
_configText = await readFile resolve(process.cwd(), 'personal-email/config.yaml'), 'utf8'
_config = YAML.parse _configText
VALID_FOLDERS = Object.keys(_config?.google_email?.labels or {})
SIMPLE_OPS = new Set ['delete', 'archive', 'skip']

###
 Validate and normalise a recommendation.operations string.

 Valid patterns (case-insensitive command; folder name matched case-insensitively,
 canonical case from config preserved):
   delete | archive | skip
   move to <folder>   — folder must be in personal-email/config.yaml google_email.labels

 Returns { valid: true, normalized } or { valid: false, reason }.
###
validateOperation = (op) ->
  s    = String(op or '').trim()
  low  = s.toLowerCase()
  return { valid: true, normalized: low } if SIMPLE_OPS.has low

  match = low.match /^move to (.+)$/
  if match
    folderLower = match[1].trim()
    canonical   = VALID_FOLDERS.find (f) -> f.toLowerCase() is folderLower
    return { valid: true, normalized: "move to #{canonical}" } if canonical
    return { valid: false, reason: "unknown folder #{JSON.stringify match[1]}; valid: #{VALID_FOLDERS.join ', '}" }

  { valid: false, reason: "must be delete | archive | skip | move to <folder>" }

###
 Trial-mode operator gate.

 1. Validates recommendation.operations against the 4 legal patterns.
    An invalid operation means the recommend microagent (01-recommend-action)
    is producing malformed output — a prompt/tool problem the coach cannot fix.
    Trial exits immediately with exit code 2; the operator should fix the
    microagent then re-run with --from-id <entity-id>.

 2. Compares the normalised given answer against the normalised correct answer:
      correct = operator_input._parsed_valid_operation  (deterministic, preferred)
              ∥ _trial.correct_answer                   (recommendation.operations from live run)

 Pass  → given === correct (after normalisation)
 Fail  → given !== correct

 Result is stored in entity.trial_result for use by the report system.
###
export operatorSystem = ->
  entities = (_G.World.Entity__find (e) -> e.recommendation? and not e.trial_result?)[0..._G.pipelineWidth]
  for entity in entities
    _G.currentEntityId = entity.id
    { recommendation, _trial, operator_input } = entity

    given = String(recommendation?.operations or '').trim()

    # --- Step 1: validate format -------------------------------------------
    givenValidation = validateOperation given
    unless givenValidation.valid
      console.error "\n❌ VALIDATION FAILURE: entity #{entity.id}"
      console.error "   recommendation.operations = #{JSON.stringify given}"
      console.error "   Reason: #{givenValidation.reason}"
      console.error "\n   The recommend microagent (01-recommend-action) produced an invalid operation."
      console.error "   Fix the microagent prompt / tool description, then re-run:"
      console.error "     bun trial-runner/agent.coffee --from-id #{entity.id}\n"
      process.exit 2

    given = givenValidation.normalized  # canonical form

    # --- Step 2: normalise correct answer ------------------------------------
    # Prefer deterministic _parsed_valid_operation; fall back to _trial.correct_answer
    rawCorrect = String(
      operator_input?._parsed_valid_operation or
      _trial?.correct_answer or ''
    ).trim()
    correctValidation = validateOperation rawCorrect
    correct = if correctValidation.valid then correctValidation.normalized else rawCorrect.toLowerCase()

    # --- Step 3: compare -----------------------------------------------------
    passed = given is correct

    await _G.Entity.patch entity, 'trial_result',
      given: given
      correct: correct
      passed: passed
      trial_rationale: _trial?.trial_rationale or ''
      original_rationale: _trial?.original_rationale or ''

    status = if passed then '✅ PASS' else '❌ FAIL'
    _G.log 'trial.operator.result', { id: entity.id, given, correct, passed }
    await _G.Entity.log entity, "#{status} given=#{JSON.stringify given} correct=#{JSON.stringify correct}"
