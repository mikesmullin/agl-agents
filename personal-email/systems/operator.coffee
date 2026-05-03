import '../microagents/02-contains-question.coffee'
import '../microagents/04-answer-question-from-email.coffee'
import '../microagents/07-execute-memo-instruction.coffee'
import { _G } from '../../lib/globals.coffee'

export operatorSystem = ->
  # Stage 1: write operator_input gate for entities ready for human review
  pending = _G.World.Entity__find (e) -> e.recommendation? and not e.operator_input? and not e.operator?
  for entity in pending
    _G.currentEntityId = entity.id
    await _G.Entity.patch entity, 'operator_input',
      instruction: null     # activation input: proceed / skip / delete / move to X / memo ... / question / etc.
      rationale: null       # why (1-2 sentences); flows into journal + seeds trial_rationale
      notice_capture: null  # (future) what information to capture for notification toaster
      notice_display: null  # (future) how to present it in the toaster summary
      recommendation: entity.recommendation.label

  # Stage 2: process entities where human has filled in an instruction
  entities = _G.World.Entity__find (e) ->
    e.operator_input? and not e.operator? and e.operator_input.instruction?
  for entity in entities
    _G.currentEntityId = entity.id
    { content, recommendation, operator_input } = entity
    instruction = String(operator_input.instruction or '').trim()
    rationale   = String(operator_input.rationale   or '').trim()
    continue unless instruction

    normalizedInstruction = instruction.toLowerCase()
    normalizedInstruction = 'proceed' if normalizedInstruction is 'p'
    commandWords = new Set normalizedInstruction.match(/[a-z]+/g) or []

    if normalizedInstruction is 'quit' or commandWords.has 'quit'
      _G.quit = true
      await _G.Entity.patch entity, 'operator', { command: 'quit', instruction: '' }
      continue

    if normalizedInstruction is 'skip'
      await _G.Entity.patch entity, 'operator', { command: 'skip', instruction: '' }
      continue

    if normalizedInstruction is 'refresh'
      await _G.Entity.patch entity, 'operator', { command: 'refresh', instruction: '' }
      continue

    if normalizedInstruction is 'reload'
      await _G.Entity.patch entity, 'operator', { command: 'reload', instruction: '' }
      continue

    if normalizedInstruction is 'proceed'
      # Compose instructionOrRecommendation with rationale so journal stage sees the human's reasoning
      instructionOrRec = if rationale
        "#{recommendation.label}. #{rationale}"
      else
        recommendation.label
      await _G.Entity.patch entity, 'operator',
        command: 'proceed'
        instruction: recommendation.label
        instructionOrRecommendation: instructionOrRec
        rationale: rationale
      continue

    if /\b(memo|memos|journal|journals)\b/i.test instruction
      memoExec = await _G.executeMemoInstructionMicroagent instruction
      entity = await _G.Entity.log entity, "memo.instruction.executed: success=#{memoExec.success} #{memoExec.summary}"
      # Clear instruction so human can provide the next one; record result
      entity = await _G.Entity.patch entity, 'operator_input',
        { ...entity.operator_input, instruction: null, last_result: memoExec.summary }
      continue

    hasQuestion = await _G.containsQuestionMicroagent instruction
    if hasQuestion
      answer = await _G.answerQuestionFromEmailMicroagent content.body, instruction
      # Write answer back to entity state; clear instruction for next round
      entity = await _G.Entity.patch entity, 'operator_input',
        { ...entity.operator_input, instruction: null, last_answer: String(answer or '') }
      continue

    # Custom instruction → proceed to execute stage; include rationale in composed string for journal
    instructionOrRec = if rationale then "#{instruction}. #{rationale}" else instruction
    await _G.Entity.patch entity, 'operator',
      command: 'execute'
      instruction: instruction
      instructionOrRecommendation: instructionOrRec
      rationale: rationale
