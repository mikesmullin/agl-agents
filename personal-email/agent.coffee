import Agent from '../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../lib/globals.coffee'
import '../lib/color.coffee'
import '../lib/debug.coffee'
import '../lib/spawn.coffee'
import '../lib/text.coffee'
import '../lib/validate.coffee'
import '../lib/voice.coffee'
import '../lib/email-adapter.coffee'
import '../lib/html-email.coffee'
import '../lib/memo.coffee'
import '../lib/recall.coffee'
import '../lib/async.coffee'
import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { mkdir } from 'fs/promises'
import { resolve } from 'path'
import '../lib/time.coffee'

import './models/world.coffee'
import './models/entity.coffee'
import { pageSystem } from './systems/page.coffee'
import { loadSystem } from './systems/load.coffee'
import { fingerprintSystem } from './systems/fingerprint.coffee'
import { recallSystem } from './systems/recall.coffee'
import { summarizeSystem } from './systems/summarize.coffee'
import { recommendSystem } from './systems/recommend.coffee'
import { displaySystem } from './systems/display.coffee'
import { operatorSystem } from './systems/operator.coffee'
import { executeSystem } from './systems/execute.coffee'
import { journalSystem } from './systems/journal.coffee'
import { planSystem } from './systems/plan.coffee'
import { applySystem } from './systems/apply.coffee'
import { cleanSystem } from './systems/cleanup.coffee'
import { resetSystem } from './systems/refresh.coffee'

Agent.default.model = _G.MODEL
Agent.default.context_window = _G.CONTEXT_WINDOW

journalConfig = {}
try
  configText = await readFile resolve(process.cwd(), 'config.yaml'), 'utf8'
  cfg = YAML.parse(configText or '') ? {}
  if cfg?.journal and typeof cfg.journal is 'object'
    journalConfig = { ...journalConfig, ...cfg.journal }
catch
  # use defaults

await _G.Entity.init()
await mkdir _G.DB_DIR, { recursive: true }
await _G.ensureJournalGitRepoLib _G.spawn, _G.DB_DIR

_sigintCount = 0
beginGracefulShutdown = ->
  _sigintCount++
  if _sigintCount >= 2
    console.log '\nForce quitting.'
    process.exit 1
  _G.quit = true
  console.log '\nGraceful shutdown requested; finishing current iteration...'

process.on 'SIGINT', beginGracefulShutdown
process.on 'SIGTERM', beginGracefulShutdown

await _G.loadMoveFolderCacheLib()

cliArgs = process.argv.slice(2)
since = String(cliArgs[0] or '').trim() or undefined

pageLoadCooldown = new _G.Cooldown 60_000*5 # 5min

while not _G.quit
  if pageLoadCooldown.tick()
    await pageSystem since

  await loadSystem()
  await fingerprintSystem()
  await recallSystem()
  await summarizeSystem()
  await recommendSystem()
  await displaySystem journalConfig.confidence_threshold
  await operatorSystem()
  await resetSystem()
  await executeSystem()
  await journalSystem()
  await planSystem()
  await applySystem()
  await cleanSystem()

  # Print a status line when all systems are idle (nothing to do this tick)
  now = Date.now()
  all = _G.World.Entity__all()
  awaitingApproval  = all.filter (e) -> e.apply?.approved is null
  awaitingOperator  = all.filter (e) -> e.operator_input?.instruction is null and not e.skip?
  skipped           = all.filter (e) -> e.skip?.active
  inBackoff         = all.filter (e) -> e._error?.nextRetryAt? and new Date(e._error.nextRetryAt).getTime() > now
  if awaitingApproval.length or awaitingOperator.length or skipped.length or inBackoff.length
    pullInSec = Math.ceil pageLoadCooldown.remain() / 1000
    parts = []
    parts.push "#{awaitingApproval.length} awaiting apply approval"  if awaitingApproval.length
    parts.push "#{awaitingOperator.length} awaiting operator input"  if awaitingOperator.length
    parts.push "#{skipped.length} skipped"                           if skipped.length
    parts.push "#{inBackoff.length} in backoff"                      if inBackoff.length
    _G.traceReplaceLine '💤', "#{parts.join ', '}. Next pull in #{pullInSec}s.", { final: true }

  await _G.sleep 1_000 # 1sec
