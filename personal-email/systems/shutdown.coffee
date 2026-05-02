import { _G } from '../../lib/globals.coffee'

export shutdownSystem = ->
  _G.log 'agent.shutdown', { reason: 'no_more_emails' }
