import { _G } from './globals.mjs';

let _activeTtsJob = null;

export const stopSpeaking = _G.stopSpeaking = () => {
  if (_activeTtsJob) {
    _activeTtsJob.kill();
    _activeTtsJob = null;
  }
};

export const speakText = _G.speakText = async (text, preset = 'alan') => {
  const spokenText = String(text || '').trim();
  if (!spokenText) {
    return;
  }

  // const trace = _G.traceStart('🗣️', 'Speaking headline aloud');
  try {
    _activeTtsJob = _G.spawn('voice', ['--gain=0.8', preset, spokenText], { scope: 'tts' });
    await _activeTtsJob;
    _activeTtsJob = null;
    // trace.traceEnd();
  } catch (error) {
    _activeTtsJob = null;
    // trace.traceFail();
    _G.log('tts.failed', {
      preset,
      text: spokenText,
      error: error?.message || String(error),
    }, 'tts');
  }
};