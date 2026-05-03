import { _G } from '../../lib/globals.coffee'

# No-op in trial mode: entities are pre-loaded from the trial entity dir.
# The real page system would pull from Gmail; here we skip that entirely.
export pageSystem = ->
  _G.log 'trial.page.noop', {}
