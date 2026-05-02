import '../microagents/00-fingerprint-email.coffee'
import { _G } from '../../lib/globals.coffee'

export fingerprintSystem = ->
  entities = _G.World.Entity__find (e) -> e.content? and not e.fingerprint?
  for entity in entities
    _G.currentEntityId = entity.id
    fingerprint = await _G.fingerprintEmailMicroagent entity.content.body
    await _G.Entity.patch entity, 'fingerprint', fingerprint
