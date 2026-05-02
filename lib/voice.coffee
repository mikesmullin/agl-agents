import { _G } from './globals.mjs'

_activeTtsJob = null

export stopSpeaking = _G.stopSpeaking = ->
  if _activeTtsJob
    _activeTtsJob.kill()
    _activeTtsJob = null

export speakText = _G.speakText = (text, preset = 'alan') ->
  spokenText = String(text or '').trim()
  return unless spokenText

  # const trace = _G.traceStart('🗣️', 'Speaking headline aloud');
  try
    _activeTtsJob = _G.spawn 'voice', ['--gain=0.7', preset, spokenText], { scope: 'tts' }
    await _activeTtsJob
    _activeTtsJob = null
    # trace.traceEnd();
  catch error
    _activeTtsJob = null
    # trace.traceFail();
    _G.log 'tts.failed',
      preset
      text: spokenText
      error: error?.message or String(error)
    , 'tts'
