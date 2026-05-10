import { writeFile } from 'fs/promises'
import { resolve } from 'path'
import { dump as yamlDump } from 'js-yaml'
import { _G } from '../../lib/globals.coffee'

# ---------------------------------------------------------------------------
# In-memory timing + coach advice tracking.
# These are module-level so they survive across loop iterations within one run.
# ---------------------------------------------------------------------------
_roundStarts  = {}   # entityId → ms timestamp when this round began
_roundTimes   = []   # all completed round durations (ms), for timing stats
_coachAdvice  = []   # { id, at, passed, rounds, advice } — latest coach outputs
_roundCounts  = {}   # entityId → total coaching rounds applied so far

MAX_COACH_LOG = 30   # entries retained in recent_coach_log in progress.yaml

export markRoundStart = (entityId) ->
  _roundStarts[entityId] = Date.now()

export markRoundEnd = (entityId) ->
  started = _roundStarts[entityId]
  if started?
    _roundTimes.push Date.now() - started
    delete _roundStarts[entityId]

export incrementRoundCount = (entityId) ->
  _roundCounts[entityId] = (_roundCounts[entityId] or 0) + 1

export recordCoachAdvice = (entityId, passed, feedback, backpropRationale) ->
  entry =
    id:      entityId
    at:      new Date().toISOString()
    passed:  passed
    rounds:  (_roundCounts[entityId] or 0) + 1   # total rounds including this one
    advice:  if not passed then (backpropRationale or feedback or null) else null
  _coachAdvice.push entry
  if _coachAdvice.length > MAX_COACH_LOG * 2
    _coachAdvice.splice 0, _coachAdvice.length - MAX_COACH_LOG

export writeProgressFile = (trialDir, runId, startedAt) ->
  entities  = _G.World.Entity__find -> true
  total     = entities.length
  # Entities that have been through at least one full round (trial_result set + coached)
  completed = entities.filter (e) -> e.trial_result? and e.coached?
  passed    = completed.filter (e) -> e.trial_result.passed
  failed    = completed.filter (e) -> not e.trial_result.passed
  # Emails still cycling (seance/coach in progress this iteration, or awaiting recall)
  inFlight  = total - completed.length

  passRate = if completed.length > 0
    Math.round passed.length / completed.length * 100
  else 0

  # Coach stats: how many coaching rounds have entities used
  # _roundCounts tracks increments for failed entities; passed-on-first-try have 0
  allRounds = Object.values _roundCounts
  needCoaching  = allRounds.filter (r) -> r > 0
  firstTryPass  = passed.filter((e) -> not (_roundCounts[e.id] > 0)).length
  maxRounds     = if needCoaching.length > 0 then Math.max.apply(null, needCoaching) else 0
  minRounds     = if needCoaching.length > 0 then Math.min.apply(null, needCoaching) else 0
  sumRounds     = needCoaching.reduce(((s, r) -> s + r), 0)
  meanRounds    = if needCoaching.length > 0
    (sumRounds / needCoaching.length).toFixed 1
  else '—'

  # Timing stats across all completed rounds
  minMs  = if _roundTimes.length > 0 then Math.min.apply(null, _roundTimes) else null
  maxMs  = if _roundTimes.length > 0 then Math.max.apply(null, _roundTimes) else null
  meanMs = if _roundTimes.length > 0
    Math.round _roundTimes.reduce(((s, t) -> s + t), 0) / _roundTimes.length
  else null

  # ETA: remaining items * mean round time (rough; ignores multi-round retries)
  etaMs  = if meanMs? and (inFlight + failed.length) > 0
    meanMs * (inFlight + failed.length)
  else null
  etaIso = if etaMs? then new Date(Date.now() + etaMs).toISOString() else null

  elapsedMs = Date.now() - new Date(startedAt).getTime()

  # Recent coach log: last MAX_COACH_LOG entries (most recent first)
  recentLog = _coachAdvice.slice(-MAX_COACH_LOG).reverse()

  progress =
    run_id:     runId
    started_at: startedAt
    updated_at: new Date().toISOString()
    elapsed_ms: elapsedMs
    summary:
      total:      total
      processed:  completed.length
      in_flight:  inFlight
      passed:     passed.length
      failed:     failed.length
      pass_rate:  passRate
    coach_stats:
      emails_needing_coaching: needCoaching.length
      first_try_pass:          firstTryPass
      rounds_min:              (if minRounds > 0 then minRounds else null)
      rounds_max:              (if maxRounds > 0 then maxRounds else null)
      rounds_mean:             (if needCoaching.length > 0 then Number(meanRounds) else null)
    timing:
      round_ms_min:  minMs
      round_ms_max:  maxMs
      round_ms_mean: meanMs
      eta_ms:        etaMs
      eta_iso:       etaIso
    recent_coach_log: recentLog

  await writeFile(
    resolve(trialDir, 'progress.yaml'),
    yamlDump(progress, { indent: 2, lineWidth: 120 }),
    'utf8'
  )
