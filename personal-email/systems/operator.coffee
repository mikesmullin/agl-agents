import { readFile } from 'fs/promises'
import { resolve } from 'path'
import { YAML } from 'bun'
import { _G } from '../../lib/globals.coffee'

# ---------------------------------------------------------------------------
# Validate + normalise an operation string against the 4 legal patterns.
# Loaded once at import time from personal-email/config.yaml.
# ---------------------------------------------------------------------------
_validFolders = []
_simpleOps    = new Set ['delete', 'archive', 'skip']
try
  _configText  = await readFile resolve(process.cwd(), 'personal-email/config.yaml'), 'utf8'
  _cfg         = YAML.parse _configText
  _validFolders = Object.keys(_cfg?.google_email?.labels or {})
catch
  # non-fatal; _parsed_valid_operation will be null when config is unreadable

_validateOp = (op) ->
  s   = String(op or '').trim()
  low = s.toLowerCase()
  return { valid: true, normalized: low } if _simpleOps.has low
  m = low.match /^move to (.+)$/
  if m
    folder = _validFolders.find (f) -> f.toLowerCase() is m[1].trim()
    return { valid: true, normalized: "move to #{folder}" } if folder
  { valid: false }

export operatorSystem = ->
  # Stage 1: write operator_input gate for entities ready for human review
  pending = (_G.World.Entity__find (e) -> e.recommendation? and not e.operator_input? and not e.skip?)[0..._G.pipelineWidth]
  for entity in pending
    _G.currentEntityId = entity.id
    await _G.Entity.patch entity, 'operator_input',
      instruction: null     # activation: proceed / skip / reset / <custom instruction>
      rationale: null       # why (1-2 sentences); flows into journal + seeds trial_rationale
      notice_capture: null  # (future) information to capture for notification toaster
      notice_display: null  # (future) how to present it in the toaster summary

  # Stage 2: process entities where human has filled in an instruction
  entities = (_G.World.Entity__find (e) ->
    e.operator_input? and not e.skip? and not e.operator_input.processed? and e.operator_input.instruction?
  )[0..._G.pipelineWidth]
  for entity in entities
    _G.currentEntityId = entity.id
    { recommendation, operator_input } = entity
    instruction = String(operator_input.instruction or '').trim()
    continue unless instruction

    normalizedInstruction = instruction.toLowerCase()
    normalizedInstruction = 'proceed' if normalizedInstruction is 'p'

    if normalizedInstruction is 'skip'
      await _G.Entity.patch entity, 'skip', { active: true }
      continue

    if normalizedInstruction is 'reset'
      continue  # resetSystem will pick this up on the same iteration

    if normalizedInstruction is 'proceed'
      rationale = String(operator_input.rationale or '').trim()
      finalInstruction = if rationale
        "#{recommendation.label}. #{rationale}"
      else
        recommendation.label
      v = _validateOp recommendation.operations
      await _G.Entity.patch entity, 'operator_input',
        { ...operator_input, instruction: finalInstruction, _parsed_operation: 'proceed',
          _parsed_valid_operation: (if v.valid then v.normalized else null), processed: true }
      continue

    # Custom instruction: pass through directly to execute stage.
    # Attempt deterministic parse so trial-runner can compare against it.
    cv = _validateOp instruction
    await _G.Entity.patch entity, 'operator_input',
      { ...operator_input, _parsed_operation: 'custom',
        _parsed_valid_operation: (if cv.valid then cv.normalized else null), processed: true }

