import { _G } from './globals.mjs';

export const speakText = _G.speakText = async (text, preset = 'alan') => {
  const spokenText = String(text || '').trim();
  if (!spokenText) {
    return;
  }

  // const trace = _G.traceStart('🗣️', 'Speaking headline aloud');
  try {
    await _G.spawn('voice', [preset, spokenText], { scope: 'tts' });
    // trace.traceEnd();
  } catch (error) {
    // trace.traceFail();
    _G.log('tts.failed', {
      preset,
      text: spokenText,
      error: error?.message || String(error),
    }, 'tts');
  }
};