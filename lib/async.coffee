import { _G } from './globals.coffee'

export sleep = _G.sleep = (ms) ->
  new Promise (resolve) -> setTimeout resolve, ms
