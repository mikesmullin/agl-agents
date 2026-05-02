import { spawn as _spawn } from 'child_process'
import { debugLog } from './debug.mjs'
import { _G } from './globals.mjs'

export spawn = _G.spawn = (cmd, args = [], options = {}) ->
  { stdio = 'pipe', assertExit0 = false, scope = 'agent' } = options
  settledResult = ->
    cmd: result.cmd
    code: result.code
    stdout: result.stdout
    stderr: result.stderr
  result =
    cmd: [cmd, ...args].join(' ')
    code: null
    stdout: ''
    stderr: ''
    promise: null
    then: (onFulfilled, onRejected) ->
      result.promise.then(onFulfilled, onRejected)
    catch: (onRejected) ->
      result.promise.catch(onRejected)
    finally: (onFinally) ->
      result.promise.finally(onFinally)
  debugLog(scope, 'spawn.start', { cmd, args, stdio, assertExit0 })
  proc = _spawn(cmd, args, { stdio })

  if proc.stdout
    proc.stdout.on 'data', (d) ->
      result.stdout += d
  if proc.stderr
    proc.stderr.on 'data', (d) ->
      result.stderr += d
  result.promise = new Promise (resolve, reject) ->
    proc.on 'close', (code) ->
      result.code = code
      debugLog scope, 'spawn.done',
        cmd: result.cmd
        code: code
        stdout: result.stdout?.slice(0, 800)
        stderr: result.stderr?.slice(0, 800)
      if assertExit0 and code isnt 0
        reject(new Error("Command failed (#{code}): #{result.cmd}\n#{result.stdout}\n#{result.stderr}"))
        return
      resolve(settledResult())
    proc.on 'error', (err) ->
      debugLog(scope, 'spawn.error', { cmd: result.cmd, error: err?.message or String(err) })
      reject(err)
  result.kill = (signal = 'SIGTERM') ->
    try
      proc.kill(signal)
    catch
      # already exited
  result
