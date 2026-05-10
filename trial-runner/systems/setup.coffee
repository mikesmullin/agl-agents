import { readdir, readFile, mkdir, writeFile } from 'fs/promises'
import { resolve } from 'path'
import { load as yamlLoad, dump as yamlDump } from 'js-yaml'
import { _G } from '../../lib/globals.coffee'

ARCHIVE_SRC = resolve process.cwd(), 'personal-email/db/_archive'
TRIAL_BASE  = resolve process.cwd(), 'personal-email/db/_archive/trial'

###
 Scan the personal-email archive for completed "proceed" entities, create a
 new numbered trial-run folder, and write stripped trial entities.

 _trial fields on each trial entity:
   correct_answer      – recommendation.operations from the original run (reward signal)
   original_rationale  – operator.rationale from the original run (human anchor; never changes)
   trial_rationale     – injected as mock journal context for stage 6; starts equal to
                         original_rationale on gen 0, then advances via coach backprop_rationale

 Generation chain:
   Before seeding from the archive entity, we check whether the most recent
   *completed* trial run already has a backprop_rationale for this entity.
   If so, that becomes trial_rationale for the new run (gen N+1).
   A trial run is "completed" when its entities folder contains at least one
   entity with a trial_result field.
###
export setupSystem = ->
  await mkdir TRIAL_BASE, { recursive: true }

  # ------------------------------------------------------------------
  # Find the most recent completed trial run and load its backprop map
  # ------------------------------------------------------------------
  existingRuns = (await readdir(TRIAL_BASE).catch(-> [])).sort()

  backpropByEntityId = {}
  for runDir in [...existingRuns].reverse()
    entitiesPath = resolve TRIAL_BASE, runDir, 'entities'
    entityFiles = await readdir(entitiesPath).catch(-> [])
    yamlFiles = entityFiles.filter (f) -> f.endsWith '.yaml'
    if not yamlFiles.length then continue

    # Check if this run is completed (any entity has trial_result)
    hasResults = false
    for file in yamlFiles
      try
        text = await readFile resolve(entitiesPath, file), 'utf8'
        e = yamlLoad(text) or {}
        if e.trial_result? then hasResults = true; break
      catch then continue

    unless hasResults then continue  # skip incomplete/failed runs

    # Load backprop_rationale from this completed run
    for file in yamlFiles
      try
        text = await readFile resolve(entitiesPath, file), 'utf8'
        e = yamlLoad(text) or {}
        if e._trial?.backprop_rationale and e.id
          backpropByEntityId[e.id] = String(e._trial.backprop_rationale).trim()
      catch then continue
    break  # only need the most recent completed run

  # ------------------------------------------------------------------
  # Create new trial run folder (max existing ID + 1, not count, so deleted
  # trial folders don't cause ID reuse)
  # ------------------------------------------------------------------
  maxId = existingRuns.reduce ((m, d) ->
    n = parseInt d, 10
    if isNaN(n) then m else Math.max(m, n)
  ), 0
  runId = String(maxId + 1).padStart 3, '0'
  trialDir = resolve TRIAL_BASE, runId
  trialEntityDir = resolve trialDir, 'entities'
  await mkdir trialEntityDir, { recursive: true }

  # ------------------------------------------------------------------
  # Read archive entities and write trial entity files
  # ------------------------------------------------------------------
  allFiles = await readdir(ARCHIVE_SRC).catch(-> [])
  yamlFiles = allFiles.filter (f) -> f.endsWith('.yaml')

  entitiesCreated = 0
  for file in yamlFiles
    text = ''
    try
      text = await readFile resolve(ARCHIVE_SRC, file), 'utf8'
    catch
      continue

    entity = try yamlLoad(text) catch then null
    unless entity and typeof entity is 'object'
      continue

    # Reward function: only train on entities where operator accepted the recommendation
    # Support both old format (operator.command) and new format (operator_input._parsed_operation)
    isProceed = entity.operator?.command is 'proceed' or entity.operator_input?._parsed_operation is 'proceed'
    unless isProceed
      _G.log 'trial.setup.skip', { id: entity.id, reason: 'operator did not proceed' }
      continue

    correctAnswer = String(entity.recommendation?.operations or '').trim()
    unless correctAnswer
      _G.log 'trial.setup.skip', { id: entity.id, reason: 'no recommendation.operations' }
      continue

    # original_rationale: human anchor — prefer new format, fall back to old, then agent rationale
    originalRationale = String(
      entity.operator_input?.rationale or
      entity.operator?.rationale or
      entity.recommendation?.rationale or ''
    ).trim()

    # trial_rationale: starts equal to original on gen 0; advances via backprop on gen N+1
    trialRationale = backpropByEntityId[entity.id] or originalRationale

    generation = if backpropByEntityId[entity.id] then 'backprop' else 'gen0'
    _G.log 'trial.setup.entity', { id: entity.id, correctAnswer, trialRationale, generation }

    trialEntity =
      id: entity.id
      origin:
        raw: entity.origin?.raw or ''
      _trial:
        correct_answer: correctAnswer
        original_rationale: originalRationale
        trial_rationale: trialRationale

    await writeFile(
      resolve(trialEntityDir, "#{entity.id}.yaml"),
      yamlDump(trialEntity, { indent: 2 }),
      'utf8'
    )
    entitiesCreated++

  _G.log 'trial.setup.done', { runId, entitiesCreated, trialDir, priorBackpropCount: Object.keys(backpropByEntityId).length }
  { runId, trialDir, trialEntityDir, entitiesCreated }
