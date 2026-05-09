import { _G } from './globals.coffee'

# Cooldown timer — based on future wall-clock time (not pausable)
#
# Usage:
#   cd = new _G.Cooldown(10_000)  # 10-second cooldown
#   if cd.tick()                  # true on the first call, then true again every 10s
#     await doSomething()

export class Cooldown
  constructor: (durationMs) ->
    @durationMs = durationMs
    @_deadline = 0  # 0 = never started (CD_canceled state)

  # Ready? (never started, or deadline passed)
  rdy: ->
    @_deadline is 0 or Date.now() >= @_deadline

  # Still waiting?
  busy: ->
    not @rdy()

  # If ready, reset and return true. Otherwise return false.
  # The core "cooldown gate" — use this in a loop.
  tick: ->
    if @rdy()
      @_deadline = Date.now() + @durationMs
      return true
    false

  # Remaining ms (0 if done)
  remain: ->
    if @rdy() then 0 else @_deadline - Date.now()

  # Cancel (considered not-started)
  cancel: ->
    @_deadline = 0

  # Force complete immediately
  complete: ->
    @_deadline = Date.now() - 1

_G.Cooldown = Cooldown
