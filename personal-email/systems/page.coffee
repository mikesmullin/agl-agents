import { YAML } from 'bun'
import { _G } from '../../lib/globals.coffee'

_lastCheckedAt = null

# Transition types that mean the email is no longer actionable as unread
REMOVE_ON_TRANSITION = new Set ['deleted', 'archived', 'read']

export pageSystem = (since) ->
  # Re-read all existing entities from disk so operator YAML edits are picked up
  existingEntities = _G.World.Entity__find -> true
  for entity in existingEntities
    await _G.Entity.load entity.id

  pullResult = await _G.pullBatchLib _G.spawn,
    since: since
    log: _G.log
    traceLabel: 'Pulling latest emails'

  now = new Date().toISOString()
  pullData = try YAML.parse(pullResult?.stdout or '') catch then { results: [], gone: [] }

  # New emails written to local cache this pull
  for item in (pullData.results or [])
    { shortId, status, transitions } = item
    if status is 'written'
      unless _G.World.Entity__find((e) -> e.id is shortId).length
        _G.log 'page.entity.new', { id: shortId }
        await _G.Entity.load shortId
    else if status is 'updated'
      for t in (transitions or [])
        if REMOVE_ON_TRANSITION.has t.type
          _G.log 'page.entity.transition', { id: shortId, type: t.type }
          await _G.Entity.delete shortId
          break

  # Emails that disappeared from the remote unread inbox
  for item in (pullData.gone or [])
    { shortId } = item
    _G.log 'page.entity.gone', { id: shortId }
    await _G.Entity.delete shortId

  _lastCheckedAt = now
