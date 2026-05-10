import Agent from '../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../lib/globals.coffee'
import '../lib/color.coffee'
import '../lib/debug.coffee'
import '../lib/spawn.coffee'
import '../lib/text.coffee'
import '../lib/validate.coffee'
import '../lib/email-adapter.coffee'
import '../lib/html-email.coffee'
import '../lib/memo.coffee'
import '../lib/recall.coffee'
import '../lib/async.coffee'
import { resolve } from 'path'
import { writeFile, unlink } from 'fs/promises'
import {
  markRoundStart, markRoundEnd, incrementRoundCount,
  recordCoachAdvice, writeProgressFile
} from './systems/progress.coffee'

# Reuse personal-email models (entity dir is overridden below before init)
import '../personal-email/models/world.coffee'
import '../personal-email/models/entity.coffee'

# Reuse personal-email inference systems
# - loadSystem:             SKIP — trial entities already have content/envelope from archive
# - fingerprintSystem:      SKIP — trial entities already have fingerprint from archive
# - recallJournalSystem:    RUN  — core trial task: can the system recall the right journal entry?
# - recallPresentationSystem: SKIP — trial entities already have presentation recall from archive
# - summarizeSystem:        SKIP — trial entities already have summary from archive
# - recommendSystem:        RUN  — core trial task: does recall produce the right recommendation?
# - displaySystem:          SKIP — offline/training mode; display output is not used
import { recallJournalSystem } from '../personal-email/systems/recall.coffee'
import { recommendSystem } from '../personal-email/systems/recommend.coffee'

# Trial-specific systems
import { setupSystem } from './systems/setup.coffee'
import { pageSystem } from './systems/page.coffee'
import { operatorSystem } from './systems/operator.coffee'
import { seanceSystem } from './systems/seance.coffee'
import { reportSystem } from './systems/report.coffee'
import { coachSystem } from './systems/coach.coffee'
import { commitDbSystem, promoteFromRunId } from './systems/commit-db.coffee'

Agent.default.model = _G.MODEL

# ---------------------------------------------------------------------------
# --promote <id>: skip pipeline, promote the specified trial, and exit
# ---------------------------------------------------------------------------

promoteIdx = process.argv.indexOf '--promote'
promoteRunId = if promoteIdx >= 0 then (process.argv[promoteIdx + 1] or '').trim() or null else null

if promoteRunId
  await promoteFromRunId promoteRunId
  process.exit 0

# ---------------------------------------------------------------------------
# --from-id <id>: focus the trial run on a single entity (for debugging a
# validation failure).  All other entities are removed from World before the
# pipeline starts, giving a clean single-entity trial run.
# ---------------------------------------------------------------------------

fromIdIdx = process.argv.indexOf '--from-id'
fromId = if fromIdIdx >= 0 then (process.argv[fromIdIdx + 1] or '').trim() or null else null

# ---------------------------------------------------------------------------
# Trial-mode spawn mock
#
# Intercepts google-email inbox view calls and returns entity.origin.raw so
# the load system can parse the email without hitting the network.
# All other spawn calls (google-email list-labels for folder cache, etc.) are
# passed through to the real spawn.  Write commands (apply, pull) are blocked.
# ---------------------------------------------------------------------------

_realSpawn = _G.spawn
_G.spawn = (cmd, args = [], options = {}) ->
  if cmd is 'google-email'
    subcmd = args[0]

    if subcmd is 'inbox' and args[1] is 'view'
      emailId = args[2]
      entity = _G.World.Entity__find((e) -> e.id is emailId)[0]
      if entity?.origin?.raw
        mock = { cmd: [cmd, ...args].join(' '), code: 0, stdout: entity.origin.raw, stderr: '' }
        mock.promise = Promise.resolve mock
        return mock

    if subcmd is 'pull' or subcmd is 'apply'
      mock = { cmd: [cmd, ...args].join(' '), code: 0, stdout: '{"results":[],"gone":[]}', stderr: '' }
      mock.promise = Promise.resolve mock
      return mock

  _realSpawn cmd, args, options

# ---------------------------------------------------------------------------
# Setup: create trial run folder and load trial entities
# ---------------------------------------------------------------------------

{ runId, trialDir, trialEntityDir, entitiesCreated } = await setupSystem()

if entitiesCreated is 0
  console.error 'No eligible entities found in personal-email/db/_archive/ (need operator_input.source=proceed or operator.command=proceed).'
  process.exit 1

# Point entity model at trial entity dir before init
_G.ENTITY_DIR = trialEntityDir
_G.ARCHIVE_DIR = trialDir  # unused in trial pipeline but required by Entity.archive

# Point memo to an isolated per-trial journal (never touches the real journal)
_G.MEMO_DB = "#{trialDir}/journal"
_G.DB_DIR   = trialDir  # temp files written here during saveJournalEntry

await _G.Entity.init()

# Apply --from-id filter: keep only the specified entity in World
if fromId
  allEntities = _G.World.Entity__find -> true
  _G.World.remove entity.id for entity in allEntities when entity.id isnt fromId
  remaining = _G.World.Entity__find -> true
  if remaining.length is 0
    console.error "No entity found with id #{JSON.stringify fromId} in #{trialEntityDir}"
    process.exit 1
  console.log "\n🔍 --from-id #{fromId}: focusing on 1 entity\n"

# Load move-folder cache (read-only call; passes through mock to real google-email)
try
  await _G.loadMoveFolderCacheLib()
catch
  # Non-fatal; valid-move-destinations will show "(none loaded)" in prompt

totalEntities = (_G.World.Entity__find -> true).length
# ---------------------------------------------------------------------------
# Lock file — lets email-trainer UI detect a running trial
# ---------------------------------------------------------------------------
LOCK_PATH = resolve process.cwd(), 'personal-email/db/_archive/trial/running.lock'
startedAt = new Date().toISOString()

try
  await writeFile LOCK_PATH,
    JSON.stringify({ pid: process.pid, run_id: runId, started_at: startedAt }),
    'utf8'
catch
  # Non-fatal; lock file is best-effort

_cleanupLock = ->
  try require('fs').unlinkSync LOCK_PATH catch

process.on 'exit',    _cleanupLock
process.on 'SIGTERM', -> process.exit 0
process.on 'SIGINT',  -> process.exit 0

console.log "\n🧪 Trial run #{runId} — #{totalEntities} entities\n"

# ---------------------------------------------------------------------------
# Main loop
#
# Flow per entity (each iteration of the outer loop processes one batch):
#
#   recallJournalSystem → recommendSystem → operatorSystem
#     → if PASS: coachSystem marks it done
#     → if FAIL: seanceSystem + coachSystem + entity reset → loops back to recall
#
# The loop exits only when every entity has trial_result.passed = true and
# has been coached.  Failed entities are reset (recall.journalContext cleared,
# pipeline components cleared) so they cycle back through recall→recommend.
# The operator can Ctrl+C at any time; reportSystem is called after clean exit.
#
# Note: the trial journal (MEMO_DB) starts empty each run.  coachSystem writes
# corrected journal entries after each batch, so later batches (and retry cycles)
# benefit from earlier coaching.
# ---------------------------------------------------------------------------

await pageSystem()   # no-op in trial mode

loop
  # Mark round starts for entities about to enter recall this iteration.
  # Only entities without journalContext and not already decided.
  for entity in _G.World.Entity__find((e) -> not e.recall?.journalContext? and not e.trial_result?)
    markRoundStart entity.id

  await recallJournalSystem()
  await recommendSystem()
  await operatorSystem()

  # Mark round ends for entities that just received a trial_result.
  for entity in _G.World.Entity__find((e) -> e.trial_result? and not e.coached?)
    markRoundEnd entity.id

  await seanceSystem()
  await coachSystem()

  # Record coach advice + increment round counts for failed entities.
  for entity in _G.World.Entity__find((e) -> e.coached? and not e.trial_result?.passed)
    recordCoachAdvice entity.id, false, entity.coached?.feedback, entity._trial?.backprop_rationale
    incrementRoundCount entity.id
  # Also record pass outcomes
  for entity in _G.World.Entity__find((e) -> e.coached? and e.trial_result?.passed)
    recordCoachAdvice entity.id, true, null, null

  # Write progress snapshot.
  await writeProgressFile trialDir, runId, startedAt

  # Reset failed entities so they cycle back through recallJournalSystem with
  # the updated journal written by coachSystem.
  failedCoached = _G.World.Entity__find (e) -> e.coached? and not e.trial_result?.passed
  for entity in failedCoached
    # Preserve recall.presentationCandidate + usePresentationPreferences; drop journalContext
    recallRest = { ...(entity.recall or {}) }
    delete recallRest.journalContext
    cleared = await _G.Entity.clearComponents entity,
      ['retrospective', 'recommendation', 'trial_result', 'seance', 'coached', 'traces']
    await _G.Entity.patch cleared, 'recall', recallRest

  # Break when all entities have passed and been coached
  total  = (_G.World.Entity__find -> true).length
  passed = (_G.World.Entity__find (e) -> e.trial_result?.passed and e.coached?).length
  break if passed is total

# Coach: generate per-row feedback + backprop rationale, write report card
result = await reportSystem trialDir, runId

console.log """

📋 Trial run #{runId} complete
   Score: #{result.scoreLabel}
   Report: #{trialDir}/report-card.md
"""

console.log '   (run with --promote <id> to back up and promote this trial\'s journal → personal-email/db/)'
