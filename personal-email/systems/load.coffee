import { _G } from '../../lib/globals.coffee'

export loadSystem = ->
  entities = _G.World.Entity__find (e) -> not e.content?
  for entity in entities
    _G.currentEntityId = entity.id
    { emailText, envelope, summaryInput } = await _G.loadDecisionEmail entity.id
    raw = String(envelope.from or '')
    match = raw.match(/<([^>]+)>/) or raw.match(/(\S+@\S+)/)
    senderEmail = (match?[1] or '').toLowerCase().trim()

    entity = await _G.Entity.patch entity, 'origin', { raw: emailText }
    entity = await _G.Entity.patch entity, 'envelope',
      from: envelope.from
      subject: envelope.subject
      date: envelope.date
      senderEmail: senderEmail
    await _G.Entity.patch entity, 'content',
      body: summaryInput
