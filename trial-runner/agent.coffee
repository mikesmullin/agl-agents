import Agent from '../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../lib/globals.coffee'
import '../lib/color.coffee'
import '../lib/debug.coffee'
import '../lib/spawn.coffee'
import '../lib/text.coffee'
import '../lib/validate.coffee'
import '../lib/email-adapter.coffee'
import '../lib/html-email.coffee'
import '../lib/async.coffee'

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
import { recallSystem } from './systems/recall.coffee'
import { operatorSystem } from './systems/operator.coffee'
import { seanceSystem } from './systems/seance.coffee'
import { reportSystem } from './systems/report.coffee'

Agent.default.model = _G.MODEL

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
  console.error 'No eligible entities found in personal-email/db/_archive/ (need operator.command=proceed).'
  process.exit 1

# Point entity model at trial entity dir before init
_G.ENTITY_DIR = trialEntityDir
_G.ARCHIVE_DIR = trialDir  # unused in trial pipeline but required by Entity.archive

await _G.Entity.init()

# Load move-folder cache (read-only call; passes through mock to real google-email)
try
  await _G.loadMoveFolderCacheLib()
catch
  # Non-fatal; valid-move-destinations will show "(none loaded)" in prompt

console.log "\n🧪 Trial run #{runId} — #{entitiesCreated} entities\n"

# ---------------------------------------------------------------------------
# Run the pipeline once (no loop — each trial run is a single pass)
# ---------------------------------------------------------------------------

await pageSystem()        # no-op
await loadSystem()        # parse origin.raw via mocked spawn
await fingerprintSystem() # real LLM inference
await recallSystem()      # mock: inject trial_rationale as journal context
await summarizeSystem()   # real LLM inference
await recommendSystem()   # real LLM inference + captures retrospective context
await displaySystem()     # deterministic log
await operatorSystem()    # pass/fail gate: compare recommendation vs correct answer

# Seance: for failed entities, replay context window to get introspective explanation
await seanceSystem()

# Coach: generate per-row feedback + backprop rationale, write report card
result = await reportSystem trialDir, runId

console.log """

📋 Trial run #{runId} complete
   Score: #{result.scoreLabel}
   Report: #{trialDir}/report-card.md
"""
