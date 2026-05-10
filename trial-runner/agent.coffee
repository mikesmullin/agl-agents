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

# Reuse personal-email models (entity dir is overridden below before init)
import '../personal-email/models/world.coffee'
import '../personal-email/models/entity.coffee'

# Reuse personal-email inference systems (fingerprint, summarize, recommend, display)
import { loadSystem } from '../personal-email/systems/load.coffee'
import { fingerprintSystem } from '../personal-email/systems/fingerprint.coffee'
import { summarizeSystem } from '../personal-email/systems/summarize.coffee'
import { recommendSystem } from '../personal-email/systems/recommend.coffee'
import { displaySystem } from '../personal-email/systems/display.coffee'

# Trial-specific systems
import { setupSystem } from './systems/setup.coffee'
import { pageSystem } from './systems/page.coffee'
import { seedJournalSystem } from './systems/seed-journal.coffee'
import { operatorSystem } from './systems/operator.coffee'
import { seanceSystem } from './systems/seance.coffee'
import { reportSystem } from './systems/report.coffee'
import { coachSystem } from './systems/coach.coffee'
import { commitDbSystem, promoteFromRunId } from './systems/commit-db.coffee'

# Real recall from personal-email (hybrid vector+keyword+sender search)
import { recallSystem } from '../personal-email/systems/recall.coffee'

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

# Trial runs a single linear pass over all entities — lift the per-stage width cap
# Point entity model at trial entity dir before init
_G.ENTITY_DIR = trialEntityDir
_G.ARCHIVE_DIR = trialDir  # unused in trial pipeline but required by Entity.archive

# Point memo to an isolated per-trial journal (never touches the real journal)
_G.MEMO_DB = "#{trialDir}/journal"
_G.DB_DIR   = trialDir  # temp files written here during saveJournalEntry

await _G.Entity.init()

# Load move-folder cache (read-only call; passes through mock to real google-email)
try
  await _G.loadMoveFolderCacheLib()
catch
  # Non-fatal; valid-move-destinations will show "(none loaded)" in prompt

console.log "\n🧪 Trial run #{runId} — #{entitiesCreated} entities (batch size: #{_G.pipelineWidth})\n"

# ---------------------------------------------------------------------------
# Run the pipeline in batches of _G.pipelineWidth entities at a time.
# Each batch goes through the full pipeline before the next batch starts.
# Each system naturally picks the next unprocessed slice via [0..._G.pipelineWidth].
# ---------------------------------------------------------------------------

await pageSystem()   # no-op

loop
  unloaded  = (_G.World.Entity__find (e) -> not e.content?).length
  remaining = (_G.World.Entity__find (e) -> e.content? and not e.trial_result?).length
  break unless unloaded > 0 or remaining > 0
  await loadSystem()          # parse next batch of origin.raw via mocked spawn
  await fingerprintSystem()   # real LLM inference
  await seedJournalSystem()   # write trial_rationale as structured journal entries to _G.MEMO_DB
  await recallSystem()        # real hybrid recall against the trial journal
  await summarizeSystem()     # real LLM inference
  await recommendSystem()     # real LLM inference + captures retrospective context
  await displaySystem()       # deterministic log
  await operatorSystem()      # pass/fail gate: compare recommendation vs correct answer
  await seanceSystem()        # for failed entities: iterative coaching loop to find working rationale
  await coachSystem()         # finalise backprop_rationale + write corrected journal entry to MEMO_DB

# Coach: generate per-row feedback + backprop rationale, write report card
result = await reportSystem trialDir, runId

console.log """

📋 Trial run #{runId} complete
   Score: #{result.scoreLabel}
   Report: #{trialDir}/report-card.md
"""

console.log '   (run with --promote <id> to back up and promote this trial\'s journal → personal-email/db/)'
