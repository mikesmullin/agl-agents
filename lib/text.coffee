import { _G } from './globals.coffee'

export xmlEscape = _G.xmlEscape = (s = '') ->
  String(s)
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')

export optionalText = _G.optionalText = (enabled, value, fallback = '') ->
  if not enabled
    return String(fallback)
  String(value or fallback)

export stristr = _G.stristr = (haystack, needle) ->
  String(haystack or '').toLowerCase().includes(String(needle or '').toLowerCase())
