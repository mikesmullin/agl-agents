import { _G } from './globals.mjs';

export const xmlEscape = _G.xmlEscape = (s = '') => {
  return String(s)
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
};

export const optionalText = _G.optionalText = (enabled, value, fallback = '') => {
  if (!enabled) {
    return String(fallback);
  }
  return String(value || fallback);
};

export const stristr = _G.stristr = (haystack, needle) => {
  return String(haystack || '').toLowerCase().includes(String(needle || '').toLowerCase());
};
