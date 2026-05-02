import { ANSI } from './color.coffee'
import { _G } from './globals.coffee'
import { appendFileSync, writeFileSync } from 'fs'
import { resolve } from 'path'
import { inspect } from 'util'

DEBUG = /^(1|true|yes|on)$/i.test(process.env.DEBUG or '')
export HUMAN_TRACE = process.env.DEBUG isnt '1'
DEBUG_LOG_PATH = resolve(process.cwd(), 'debug.log')
activeTrace = null

if DEBUG
  writeFileSync(DEBUG_LOG_PATH, '')

formatLogData = (data) ->
  if typeof data is 'undefined'
    return ''

  if typeof data is 'string'
    return data

  inspect data,
    depth: null
    colors: false
    maxArrayLength: null
    maxStringLength: null
    breakLength: 120
    compact: false
    sorted: true

export debugLog = _G.debugLog = (scope, message, data) ->
  if not DEBUG then return
  prefix = "[#{new Date().toISOString()}] [#{scope}] #{message}"
  suffix = formatLogData(data)
  entry = if suffix then "#{prefix}\n#{suffix}\n" else "#{prefix}\n"
  appendFileSync(DEBUG_LOG_PATH, entry)

export log = _G.log = (message, data, scope = 'agent') ->
  debugLog(scope, message, data)

export completedLabel = _G.completedLabel = (label) ->
  map =
    'Pulling ':     'Pulled '
    'Loading ':     'Loaded '
    'Refreshing ':  'Refreshed '
    'Reloading ':   'Reloaded '
    'Searching ':   'Searched '
    'Extracting ':  'Extracted '
    'Pre-filtering ': 'Pre-filtered '
    'Summarizing ': 'Summarized '
    'Generating ':  'Generated '
    'Checking ':    'Checked '
    'Analyzing ':   'Analyzed '
    'Executing ':   'Executed '
    'Planning ':    'Planned '
    'Applying ':    'Applied '
    'Cleaning ':    'Cleaned '
    'Building ':    'Built '
    'Saving ':      'Saved '

  for prefix of map
    if label.startsWith(prefix)
      return "#{map[prefix]}#{label.slice(prefix.length)}."

  "#{label} done."

_entityIdColor = (id) ->
  hash = 0
  hash = (hash + c.charCodeAt(0)) for c in id
  t = (hash % 100) / 100
  ts = Math.abs(t - 0.5)
  h = 360 * t - 100
  s = 1.5 - 1.5 * ts
  l = 0.8 - 0.9 * ts
  hue = h * Math.PI / 180
  x = l**3 * (s * Math.cos(hue) + 1)
  y = l**3 * s * Math.sin(hue)
  z = l**3
  r = Math.max 0, Math.min 255, Math.round 255 * (0.787*x - 0.213*y)
  g = Math.max 0, Math.min 255, Math.round 255 * (-0.393*x + 0.715*y - 0.072*z)
  b = Math.max 0, Math.min 255, Math.round 255 * (-0.072*y + z)
  "\x1b[38;2;#{r};#{g};#{b}m"

export traceReplaceLine = _G.traceReplaceLine = (emoji, text, { final = false, failed = false, ms = null } = {}) ->
  if not HUMAN_TRACE then return

  now = new Date()
  timeStr = "#{String(now.getHours()).padStart 2, '0'}:#{String(now.getMinutes()).padStart 2, '0'}:#{String(now.getSeconds()).padStart 2, '0'}.#{String(now.getMilliseconds()).padStart 3, '0'}"
  entityId = _G.currentEntityId
  entityPrefix = if entityId then " #{_entityIdColor entityId}[#{entityId}]#{ANSI.reset}" else ''
  color = if failed then ANSI.red else ANSI.cyan
  detail = if ms is null then '' else " #{ANSI.dim}(#{ms} ms)#{ANSI.reset}"
  line = "#{ANSI.dim}#{timeStr}#{ANSI.reset}#{entityPrefix} #{color}#{ANSI.bold}#{emoji}#{ANSI.reset} #{color}#{text}#{ANSI.reset}#{detail}"

  if process.stdout?.isTTY
    prefix = '\r\x1b[2K'
    process.stdout.write(if final then "#{prefix}#{line}\n" else "#{prefix}#{line}")
    return

  if final
    console.log(line)

export printRobotAnswer = _G.printRobotAnswer = (answer) ->
  text = String(answer or '').trim() or 'I could not determine an answer from this email.'
  # await _G.speakText(text) # speak (backgrounded, parallelized)
  console.log("#{ANSI.cyan}#{ANSI.bold}🗣️ Answer:#{ANSI.reset} #{ANSI.cyan}#{text}#{ANSI.reset}")

export traceStart = _G.traceStart = (emoji, label) ->
  trace =
    emoji: emoji
    label: label
    started: Date.now()
    ended: false
    traceEnd: ->
      if this.ended or not HUMAN_TRACE then return
      this.ended = true
      if activeTrace is this
        activeTrace = null
      traceReplaceLine this.emoji, completedLabel(this.label),
        final: true
        ms: Date.now() - this.started
    traceFail: ->
      if this.ended or not HUMAN_TRACE then return
      this.ended = true
      if activeTrace is this
        activeTrace = null
      traceReplaceLine this.emoji, "#{this.label} failed.",
        final: true
        failed: true
        ms: Date.now() - this.started

  if not HUMAN_TRACE then return trace
  activeTrace = trace
  traceReplaceLine(emoji, "#{label}...")
  trace

export traceStep = _G.traceStep = (emoji, label, fn) ->
  trace = traceStart(emoji, label)
  try
    result = await fn()
    trace.traceEnd()
    return result
  catch err
    trace.traceFail()
    throw err

_G.HUMAN_TRACE = HUMAN_TRACE
