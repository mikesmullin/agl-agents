import { copyFile, readFile } from 'fs/promises'
import { resolve } from 'path'
import { _G } from '../../lib/globals.coffee'

PERSONAL_EMAIL_DB = resolve process.cwd(), 'personal-email/db'
TRIAL_BASE        = resolve process.cwd(), 'personal-email/db/_archive/trial'

parseScore = (md) ->
  m = md.match /\*\*Score:\*\*\s+\(\d+\/\d+\)\s+(\d+)%/
  if m then parseInt(m[1], 10) else null

###
 Commit-DB system — runs after a successful trial when --promote is passed.

 1. `git commit -am` in personal-email/db to back up the current production memo files.
 2. Copies trialDir/journal.memo + journal.yaml → personal-email/db/.

 Only journal files are promoted — the trial pipeline does not train presentation,
 so personal-email/db/presentation.memo is left untouched.
###
export commitDbSystem = (trialDir, runId, currentPct) ->

  # ------------------------------------------------------------------
  # Git commit — back up current production memo files
  # ------------------------------------------------------------------
  commitMsg = "trial-runner: backup before applying trial #{runId} (score #{currentPct}%)"
  gitResult = await _G.spawn('git', ['-C', PERSONAL_EMAIL_DB, 'commit', '-am', commitMsg]).promise

  nothingToCommit = (gitResult.stdout + gitResult.stderr).includes 'nothing to commit'
  if gitResult.code isnt 0 and not nothingToCommit
    _G.log 'trial.commitDb.gitError', { code: gitResult.code, stderr: gitResult.stderr }
    console.log "\n⚠️  git commit failed (code #{gitResult.code}): #{gitResult.stderr or gitResult.stdout}\n"
    # Non-fatal — still promote; operator can commit manually
  else if nothingToCommit
    _G.log 'trial.commitDb.gitClean', { msg: 'nothing to commit; no backup needed' }
  else
    _G.log 'trial.commitDb.gitCommit', { commitMsg }

  # ------------------------------------------------------------------
  # Promote trial journal files → production
  # ------------------------------------------------------------------
  for file in ['journal.memo', 'journal.yaml']
    src = resolve trialDir, file
    dst = resolve PERSONAL_EMAIL_DB, file
    await copyFile src, dst
    _G.log 'trial.commitDb.copy', { src, dst }

  console.log "\n✅  Promoted trial #{runId} journal → personal-email/db/ (#{currentPct}%)\n"
  { promoted: true, currentPct }

###
 Standalone promote — resolves trial dir from runId, reads score from report-card.md,
 then delegates to commitDbSystem. Used when --promote <id> is passed directly.
###
export promoteFromRunId = (runId) ->
  trialDir = resolve TRIAL_BASE, runId
  pct = 0
  try
    md = await readFile resolve(trialDir, 'report-card.md'), 'utf8'
    pct = parseScore(md) ? 0
  catch
    console.error "⚠️  Could not read report-card.md for trial #{runId} — promoting with score 0%"
  await commitDbSystem trialDir, runId, pct
