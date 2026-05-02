###
 lib/perception.mjs

 Lightweight client for the perception-voice Unix socket server.
 Mirrors the IPC protocol used by whisper/perception_client.py:
   - 4-byte big-endian length prefix + JSON payload
   - Commands: { command: "set", uid } and { command: "get", uid }

 Also implements the same word-mapping normalization and pause-buffer
 logic as whisper's VoiceKeyboard / KeyboardTyper, so spoken phrases
 are resolved to their mapped values before being returned to callers.
###

import net from 'net'
import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { resolve } from 'path'

HEADER_SIZE = 4
UID = 'personal-email-agent'
POLL_INTERVAL_MS = 300
PAUSE_THRESHOLD_S = 0.8  # same default as whisper config

# ---------------------------------------------------------------------------
# Normalize a phrase for exact-match lookup (alpha only, single spaces)
# ---------------------------------------------------------------------------
normalize = (text) ->
  text.toLowerCase().replace(/[^a-z]+/g, ' ').replace(/\s+/g, ' ').trim()

# ---------------------------------------------------------------------------
# Low-level socket helpers (mirrors perception_client.py framing)
# ---------------------------------------------------------------------------
socketRequest = (socketPath, message) ->
  new Promise (resolve, reject) ->
    sock = net.createConnection(socketPath)
    chunks = []
    headerBuf = null
    expectedLen = 0
    received = 0

    sock.on 'error', reject

    sock.on 'data', (chunk) ->
      chunks.push(chunk)
      received += chunk.length
      buf = Buffer.concat(chunks)

      if not headerBuf and buf.length >= HEADER_SIZE
        expectedLen = buf.readUInt32BE(0)
        headerBuf = true

      if headerBuf and buf.length >= HEADER_SIZE + expectedLen
        sock.destroy()
        try
          payload = buf.slice(HEADER_SIZE, HEADER_SIZE + expectedLen).toString('utf8')
          resolve(JSON.parse(payload))
        catch e
          reject(e)

    sock.on 'connect', ->
      payload = Buffer.from(JSON.stringify(message), 'utf8')
      header = Buffer.allocUnsafe(HEADER_SIZE)
      header.writeUInt32BE(payload.length, 0)
      sock.write(Buffer.concat([header, payload]))

setReadMarker = (socketPath) ->
  try
    resp = await socketRequest(socketPath, { command: 'set', uid: UID })
    return resp?.status is 'ok'
  catch
    return false

getTranscriptions = (socketPath) ->
  try
    resp = await socketRequest(socketPath, { command: 'get', uid: UID })
    if resp?.status isnt 'ok' or not resp.text then return []
    return resp.text.trim().split('\n').filter(Boolean).map((line) ->
      try
        return JSON.parse(line)
      catch
        return null
    ).filter(Boolean)
  catch
    return []

# ---------------------------------------------------------------------------
# PerceptionInput — wraps everything into a readline-compatible ask()
# ---------------------------------------------------------------------------
export class PerceptionInput
  constructor: ({ socketPath, wordMappings = {}, pauseThresholdS = PAUSE_THRESHOLD_S }) ->
    @socketPath = socketPath
    @pauseThresholdS = pauseThresholdS
    @_cancelActiveQuestion = null
    # Build normalized map
    @_mappings = {}
    for [phrase, output] in Object.entries(wordMappings)
      @_mappings[normalize(phrase)] = output

  ### Resolve a buffered utterance string to its mapped value, or null. ###
  _resolve: (text) ->
    @_mappings[normalize(text)] ? null

  ###
   Set the read marker so future get() calls only return new utterances.
   Call this just before displaying a prompt.
  ###
  setMarker: ->
    await setReadMarker(@socketPath)

  cancelActiveQuestion: ->
    if typeof @_cancelActiveQuestion is 'function'
      @_cancelActiveQuestion()

  ###
   Display `promptText` to stdout, then poll perception-voice until a
   word-mapped utterance arrives.  Returns the mapped value (e.g. "proceed\n").

   While polling, any unmapped utterance is printed as a hint so the operator
   can see what was heard but not matched.
  ###
  question: (promptText, rl) ->
    @cancelActiveQuestion()

    process.stdout.write(promptText)
    await setReadMarker(@socketPath)

    pendingTexts = []
    pendingLastTs = null
    spokenBuffer = ''
    settled = false
    polling = false

    appendSpoken = (text) ->
      trimmed = String(text or '').trim()
      if not trimmed then return
      spokenBuffer = if spokenBuffer then "#{spokenBuffer} #{trimmed}" else trimmed

    flushPending = ->
      if not pendingTexts.length then return ''
      buffered = pendingTexts.join(' ').trim()
      pendingTexts = []
      pendingLastTs = null
      return buffered

    refreshPrompt = ->
      process.stdout.write(promptText)

    emitHeard = (text, suffix = '') ->
      process.stdout.write("\n  [heard: \"#{text}\"#{suffix}]\n")

    new Promise (resolvePromise, rejectPromise) =>
      controller = new AbortController()
      timer = null
      cancelledError = -> Object.assign(new Error('Perception prompt cancelled'), { code: 'ABORT_ERR' })
      isPromptUnavailable = -> settled or controller.signal.aborted or rl?.closed

      cleanup = =>
        if timer
          clearInterval(timer)
          timer = null
        controller.abort()
        if @_cancelActiveQuestion is cancel
          @_cancelActiveQuestion = null

      resolveWith = (value) ->
        if settled then return
        settled = true
        cleanup()
        resolvePromise(value)

      rejectWith = (error) ->
        if settled then return
        settled = true
        cleanup()
        rejectPromise(error)

      combineInput = (typedText) ->
        typed = String(typedText or '')
        if not spokenBuffer then return typed.trim()
        if not typed then return spokenBuffer.trim()
        if spokenBuffer.endsWith('\n') or typed.startsWith('\n')
          return "#{spokenBuffer}#{typed}".trim()
        return "#{spokenBuffer} #{typed}".trim()

      startKeyboardQuestion = ->
        if isPromptUnavailable()
          rejectWith(cancelledError())
          return
        rl.question('', { signal: controller.signal })
          .then(finalizeInput)
          .catch (error) ->
            if error?.name is 'AbortError' or error?.code is 'ABORT_ERR'
              rejectWith(cancelledError())
              return
            if error?.code is 'ERR_USE_AFTER_CLOSE' and (rl?.closed or controller.signal.aborted)
              rejectWith(cancelledError())
              return
            rejectWith(error)

      finalizeInput = (typedText) ->
        pending = flushPending()
        if pending
          emitHeard(pending)
          appendSpoken(pending)
        finalInput = combineInput(typedText)
        if not finalInput
          if isPromptUnavailable()
            rejectWith(cancelledError())
            return
          refreshPrompt()
          startKeyboardQuestion()
          return
        resolveWith(finalInput)

      handleFlushedUtterance = (buffered) =>
        if not buffered then return

        normalized = normalize(buffered)
        mapped = @_resolve(buffered)
        if mapped isnt null
          mappedValue = mapped.replace(/\\n/g, '\n')
          if normalized is 'erase'
            spokenBuffer = ''
            pendingTexts = []
            pendingLastTs = null
            try
              rl?.write(null, { ctrl: true, name: 'u' })
            catch
            emitHeard(buffered, ' - cleared input')
            refreshPrompt()
            return
          if normalized is 'submit'
            emitHeard(buffered, ' - submit')
            finalizeInput('')
            return
          if mappedValue is '\n'
            emitHeard(buffered)
            spokenBuffer = if spokenBuffer then "#{spokenBuffer}\n" else '\n'
            refreshPrompt()
            return

          process.stdout.write("#{buffered}\n")
          resolveWith(mappedValue.trimEnd())
          return

        emitHeard(buffered)
        appendSpoken(buffered)
        refreshPrompt()

      cancel = ->
        rejectWith(cancelledError())
      @_cancelActiveQuestion = cancel

      startKeyboardQuestion()

      timer = setInterval (=>
        if settled or polling then return
        polling = true
        try
          items = await getTranscriptions(@socketPath)

          for item in items
            text = String(item.text or '').trim()
            if not text then continue

            try
              itemTs = new Date(item.ts).getTime() / 1000
            catch
              itemTs = Date.now() / 1000

            if pendingLastTs isnt null and itemTs - pendingLastTs >= @pauseThresholdS
              handleFlushedUtterance(flushPending())
              if settled then return

            pendingTexts.push(text)
            pendingLastTs = itemTs

          if pendingLastTs isnt null and Date.now() / 1000 - pendingLastTs >= @pauseThresholdS
            handleFlushedUtterance(flushPending())
        finally
          polling = false
      ), POLL_INTERVAL_MS

# ---------------------------------------------------------------------------
# Load perception config from personal-email/config.yaml under perception: key
# ---------------------------------------------------------------------------
export loadPerceptionConfig = (cwd) ->
  configPath = resolve(cwd, 'personal-email', 'config.yaml')
  text = await readFile(configPath, 'utf8')
  file = YAML.parse(text) ? {}
  cfg = if (file.perception and typeof file.perception is 'object') then file.perception else {}
  socketPath = resolve(cwd, String(cfg.socket_path or 'perception.sock'))
  wordMappings = if cfg.word_mappings and typeof cfg.word_mappings is 'object'
    cfg.word_mappings
  else
    {}
  pauseThresholdS = Number(cfg.pause_threshold_s ? PAUSE_THRESHOLD_S)
  return new PerceptionInput({ socketPath, wordMappings, pauseThresholdS })
