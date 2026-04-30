import { YAML } from 'bun';
import { _G } from './globals.mjs';



function normalizeEmailDoc(parsed) {
  if (Array.isArray(parsed)) {
    const docs = parsed.filter((d) => d && typeof d === 'object' && !Array.isArray(d));
    if (docs.length === 0) return parsed[0] || {};
    const merged = {};
    for (const d of docs) {
      Object.assign(merged, d);
    }
    return merged;
  }
  if (parsed && typeof parsed === 'object') {
    return parsed;
  }
  return {};
}

function decodeHtmlEntities(text) {
  if (!text) return '';

  const named = {
    amp: '&',
    lt: '<',
    gt: '>',
    quot: '"',
    apos: "'",
    nbsp: ' ',
    shy: '',
    ndash: '-',
    mdash: '-',
    hellip: '...',
  };

  let decoded = String(text);
  for (let pass = 0; pass < 3; pass += 1) {
    const next = decoded.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (m, entity) => {
      if (!entity) return m;

      if (entity[0] === '#') {
        const isHex = entity[1]?.toLowerCase() === 'x';
        const raw = isHex ? entity.slice(2) : entity.slice(1);
        const num = parseInt(raw, isHex ? 16 : 10);
        if (Number.isFinite(num) && num > 0) {
          try {
            return String.fromCodePoint(num);
          }
          catch {
            return m;
          }
        }
        return m;
      }

      const key = entity.toLowerCase();
      return Object.prototype.hasOwnProperty.call(named, key) ? named[key] : m;
    });

    if (next === decoded) {
      break;
    }
    decoded = next;
  }

  return decoded;
}

function pickAttr(tagText, attrName) {
  const re = new RegExp(`${attrName}\\s*=\\s*(["'])(.*?)\\1`, 'i');
  const match = String(tagText || '').match(re);
  return decodeHtmlEntities(match?.[2] || '').trim();
}

function normalizeWhitespace(text) {
  return String(text || '')
    .replace(/[\u00AD\u034F\u061C\u115F\u1160\u17B4\u17B5\u180E\u2000-\u200F\u2028-\u202F\u205F\u2060-\u206F\u3000\uFE00-\uFE0F\uFEFF]/g, ' ')
    .replace(/\r/g, '')
    .replace(/[ \t\f\v]+/g, ' ')
    .replace(/ *\n */g, '\n')
    .replace(/([^\w\n])\1{8,}/g, '$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function htmlToReadableText(html) {
  let s = String(html || '');
  if (!s.trim()) return '';

  // Preserve accessibility-relevant attributes before stripping tags.
  s = s.replace(/<img\b[^>]*>/gi, (tag) => {
    const alt = pickAttr(tag, 'alt') || pickAttr(tag, 'aria-label') || pickAttr(tag, 'title');
    return alt ? ` [Image: ${alt}] ` : ' ';
  });

  s = s.replace(/<a\b[^>]*>([\s\S]*?)<\/a>/gi, (full, inner) => {
    const tagStart = full.match(/<a\b[^>]*>/i)?.[0] || '';
    const label = pickAttr(tagStart, 'aria-label') || pickAttr(tagStart, 'title');
    const href = pickAttr(tagStart, 'href');

    const innerText = String(inner || '').replace(/<[^>]+>/g, ' ').trim();
    const chosen = decodeHtmlEntities(innerText || label || href || '').trim();
    return chosen ? ` ${chosen} ` : ' ';
  });

  s = s
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, ' ')
    .replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, ' ')
    .replace(/<noscript\b[^>]*>[\s\S]*?<\/noscript>/gi, ' ')
    .replace(/<head\b[^>]*>[\s\S]*?<\/head>/gi, ' ')
    .replace(/<svg\b[^>]*>[\s\S]*?<\/svg>/gi, ' ')
    .replace(/<br\s*\/?\s*>/gi, '\n')
    .replace(/<\/(p|div|li|tr|h[1-6]|section|article|header|footer)>/gi, '\n')
    .replace(/<[^>]+>/g, ' ');

  s = decodeHtmlEntities(s);
  s = normalizeWhitespace(s);
  return s;
}

function compact(text, maxChars) {
  const cleaned = normalizeWhitespace(text);
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, maxChars - 3).trimEnd()}\b...SNIP...`;
}

export const prefilterEmailForSummary = _G.prefilterEmailForSummary = (emailYamlText) => {
  let doc = {};
  try {
    const parsed = YAML.parse(String(emailYamlText || ''));
    doc = normalizeEmailDoc(parsed);
  }
  catch {
    return compact(decodeHtmlEntities(String(emailYamlText || '')), _G.EMAIL_BODY_MAX_BYTES);
  }

  const snippet = decodeHtmlEntities(String(doc?.snippet || '').trim());

  const bodyType = String(doc?.body?.contentType || '').toLowerCase();
  const bodyRaw = String(doc?.body?.content || '');
  const bodyText = bodyType === 'html' ? htmlToReadableText(bodyRaw) : normalizeWhitespace(decodeHtmlEntities(bodyRaw));

  const lines = [
    snippet ? `Snippet: ${snippet}` : '',
    '',
    'Body:',
    bodyText,
  ].filter(Boolean);

  return compact(lines.join('\n'), _G.EMAIL_BODY_MAX_BYTES);
};
