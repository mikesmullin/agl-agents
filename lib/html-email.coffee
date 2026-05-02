import { YAML } from 'bun'
import { _G } from './globals.coffee'



normalizeEmailDoc = (parsed) ->
  if Array.isArray(parsed)
    docs = parsed.filter((d) -> d and typeof d is 'object' and not Array.isArray(d))
    if docs.length is 0
      return parsed[0] or {}
    merged = {}
    for d in docs
      Object.assign(merged, d)
    return merged
  if parsed and typeof parsed is 'object'
    return parsed
  {}

decodeHtmlEntities = (text) ->
  if not text then return ''

  named =
    amp: '&'
    lt: '<'
    gt: '>'
    quot: '"'
    apos: "'"
    nbsp: ' '
    shy: ''
    ndash: '-'
    mdash: '-'
    hellip: '...'

  decoded = String(text)
  for pass in [0...3] by 1
    next = decoded.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (m, entity) ->
      if not entity then return m

      if entity[0] is '#'
        isHex = entity[1]?.toLowerCase() is 'x'
        raw = if isHex then entity.slice(2) else entity.slice(1)
        num = parseInt(raw, if isHex then 16 else 10)
        if Number.isFinite(num) and num > 0
          try
            return String.fromCodePoint(num)
          catch
            return m
        return m

      key = entity.toLowerCase()
      if Object.prototype.hasOwnProperty.call(named, key) then named[key] else m
    )

    if next is decoded
      break
    decoded = next

  decoded

pickAttr = (tagText, attrName) ->
  re = new RegExp("#{attrName}\\s*=\\s*([\"'])(.*?)\\1", 'i')
  match = String(tagText or '').match(re)
  decodeHtmlEntities(match?[2] or '').trim()

normalizeWhitespace = (text) ->
  String(text or '')
    .replace(/[­͏؜ᅟᅠ឴឵᠎ -‏ -  ⁠-⁯　︀-️﻿]/g, ' ')
    .replace(/\r/g, '')
    .replace(/[ \t\f\v]+/g, ' ')
    .replace(/ *\n */g, '\n')
    .replace(/([^\w\n])\1{8,}/g, '$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim()

htmlToReadableText = (html) ->
  s = String(html or '')
  if not s.trim() then return ''

  # Preserve accessibility-relevant attributes before stripping tags.
  s = s.replace(/<img\b[^>]*>/gi, (tag) ->
    alt = pickAttr(tag, 'alt') or pickAttr(tag, 'aria-label') or pickAttr(tag, 'title')
    if alt then " [Image: #{alt}] " else ' '
  )

  s = s.replace(/<a\b[^>]*>([\s\S]*?)<\/a>/gi, (full, inner) ->
    tagStart = full.match(/<a\b[^>]*>/i)?[0] or ''
    label = pickAttr(tagStart, 'aria-label') or pickAttr(tagStart, 'title')
    href = pickAttr(tagStart, 'href')

    innerText = String(inner or '').replace(/<[^>]+>/g, ' ').trim()
    chosen = decodeHtmlEntities(innerText or label or href or '').trim()
    if chosen then " #{chosen} " else ' '
  )

  s = s
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, ' ')
    .replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, ' ')
    .replace(/<noscript\b[^>]*>[\s\S]*?<\/noscript>/gi, ' ')
    .replace(/<head\b[^>]*>[\s\S]*?<\/head>/gi, ' ')
    .replace(/<svg\b[^>]*>[\s\S]*?<\/svg>/gi, ' ')
    .replace(/<br\s*\/?\s*>/gi, '\n')
    .replace(/<\/(p|div|li|tr|h[1-6]|section|article|header|footer)>/gi, '\n')
    .replace(/<[^>]+>/g, ' ')

  s = decodeHtmlEntities(s)
  s = normalizeWhitespace(s)
  s

compact = (text, maxChars) ->
  cleaned = normalizeWhitespace(text)
  if cleaned.length <= maxChars then return cleaned
  "#{cleaned.slice(0, maxChars - 3).trimEnd()}\b...SNIP..."

export prefilterEmailForSummary = _G.prefilterEmailForSummary = (emailYamlText) ->
  doc = {}
  try
    parsed = YAML.parse(String(emailYamlText or ''))
    doc = normalizeEmailDoc(parsed)
  catch
    return compact(decodeHtmlEntities(String(emailYamlText or '')), _G.EMAIL_BODY_MAX_BYTES)

  snippet = decodeHtmlEntities(String(doc?.snippet or '').trim())

  bodyType = String(doc?.body?.contentType or '').toLowerCase()
  bodyRaw = String(doc?.body?.content or '')
  bodyText = if bodyType is 'html' then htmlToReadableText(bodyRaw) else normalizeWhitespace(decodeHtmlEntities(bodyRaw))

  lines = [
    if snippet then "Snippet: #{snippet}" else ''
    ''
    'Body:'
    bodyText
  ].filter(Boolean)

  compact(lines.join('\n'), _G.EMAIL_BODY_MAX_BYTES)
