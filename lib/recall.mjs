import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { _G } from './globals.mjs';

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Parse a multi-document YAML file (memo journal format) into an array of
 * entry objects: { id, metadata, body, fields }.
 */
async function readJournalEntries(memoDbPath) {
  let raw = '';
  try {
    raw = await readFile(`${memoDbPath}.yaml`, 'utf8');
  }
  catch {
    return [];
  }

  const docs = raw.split(/(?:^|\n)---(?:\n|$)/).map((s) => s.trim()).filter(Boolean);
  const entries = [];

  for (const doc of docs) {
    try {
      const parsed = YAML.parse(doc) || {};
      if (!parsed || typeof parsed !== 'object') continue;
      const id = Number(parsed.id ?? -1);
      if (!Number.isFinite(id) || id < 0) continue;
      const body = String(parsed.body || '');
      entries.push({
        id,
        metadata: parsed.metadata || {},
        body,
        fields: parseBodyFields(body),
      });
    }
    catch {
      // skip malformed documents
    }
  }

  return entries;
}

/** Parse the key: value lines in a journal entry body into a plain object. */
function parseBodyFields(bodyText) {
  const fields = {};
  let currentKey = '';
  for (const rawLine of String(bodyText || '').split('\n')) {
    const line = rawLine.replace(/\r$/, '');
    const kv = line.match(/^\s*([a-zA-Z_][a-zA-Z0-9_-]*):\s*(.*)$/);
    if (kv) {
      currentKey = kv[1];
      fields[currentKey] = String(kv[2] || '').trim();
      continue;
    }
    if (currentKey && /^\s+\S/.test(line)) {
      fields[currentKey] = `${fields[currentKey]} ${line.trim()}`.trim();
    }
  }
  return fields;
}

/** Format a raw entry as context text for the LLM, matching buildJournalContext format. */
function formatEntryAsContext(entry) {
  const f = entry.fields;
  return JSON.stringify({
    id: entry.id,
    summary: f.summary || '',
    keywords: Array.isArray(entry.metadata?.keywords) ? entry.metadata.keywords.join(', ') : '',
    action_taken: f.action_taken || '',
    factors: f.factors || '',
    rule: f.rule || '',
    applies_if: f.applies_if || '',
    match_criteria: f.match_criteria || '',
    sender_email: String(entry.metadata?.sender_email || ''),
    raw_excerpt: entry.body.slice(0, 1200),
  });
}

// ---------------------------------------------------------------------------
// Strategy 1 — Exact sender match (deterministic)
// ---------------------------------------------------------------------------

async function recallBySender(spawn, memoDbPath, incomingSenderEmail) {
  const senderEmail = String(incomingSenderEmail || '').toLowerCase().trim();
  if (!senderEmail) return [];

  const filter = `sender_email: ${JSON.stringify(senderEmail)}`;
  const result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--filter', filter, '--yaml', senderEmail]);
  if (result.code !== 0) return [];
  return _G.parseMemoRecallResults(result.stdout);
}

// ---------------------------------------------------------------------------
// Strategy 2 — Keyword full-text search (hybrid)
// ---------------------------------------------------------------------------

async function recallByKeywords(spawn, memoDbPath, fingerprint, emailText) {
  const rawKw = String(fingerprint?.keywords || '')
    .split(',')
    .map((k) => k.trim().toLowerCase())
    .filter(Boolean);

  if (!rawKw.length) return [];

  const uniqueKeywords = [...new Set(rawKw)].slice(0, 8);
  const results = await Promise.all(uniqueKeywords.map(async (keyword) => {
    const filter = `keywords: {$contains: ${JSON.stringify(keyword)}}`;
    const r = await spawn('memo', [
      'recall', '-f', memoDbPath, '-k', '5', '--filter', filter, '--yaml', emailText.slice(0, 1500),
    ]);
    return r.code === 0 ? _G.parseMemoRecallResults(r.stdout) : [];
  }));

  const byId = new Map();
  for (const rows of results) {
    for (const row of rows) {
      if (row.id <= 0) continue;
      const existing = byId.get(row.id);
      if (!existing || row.score > existing.score) {
        byId.set(row.id, row);
      }
    }
  }

  return [...byId.values()].sort((a, b) => b.score - a.score).slice(0, 5);
}

// ---------------------------------------------------------------------------
// Strategy 3 — Q&A semantic search via memo recall
// ---------------------------------------------------------------------------

async function recallByQA(spawn, memoDbPath, fingerprint) {
  const query = [
    fingerprint?.sender_offers,
    fingerprint?.sender_expects,
    fingerprint?.reader_value,
  ].filter((s) => s && s !== 'none').join('. ');

  if (!query.trim()) return [];

  const result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--yaml', query]);
  if (result.code !== 0) return [];
  return _G.parseMemoRecallResults(result.stdout);
}

// ---------------------------------------------------------------------------
// Strategy 4 — Whole-email vector search (existing)
// ---------------------------------------------------------------------------

async function recallByVector(spawn, memoDbPath, emailText) {
  const query = emailText.slice(0, 1500);
  const result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--yaml', query]);
  if (result.code !== 0) return [];
  return _G.parseMemoRecallResults(result.stdout);
}

// ---------------------------------------------------------------------------
// Result fusion
// ---------------------------------------------------------------------------

function fuseResults(allEntries, s1Entries, s2Entries, s3Results, s4Results) {
  // Build a lookup map by ID from raw journal
  const byId = new Map(allEntries.map((e) => [e.id, e]));

  // Collect candidate IDs and count how many strategies returned each
  const strategyCounts = new Map();
  const add = (id) => strategyCounts.set(id, (strategyCounts.get(id) || 0) + 1);

  for (const r of s1Entries) if (r.id > 0) add(r.id);
  for (const r of s2Entries) if (r.id > 0) add(r.id);
  for (const r of s3Results) if (r.id > 0) add(r.id);
  for (const r of s4Results) if (r.id > 0) add(r.id);

  // Build unified sorted list
  const candidates = [...strategyCounts.entries()]
    .map(([id, strategies]) => {
      const entry = byId.get(id);
      return {
        id,
        strategies,
        confirmedCount: Number(entry?.metadata?.confirmed_count || 0),
        entry,
      };
    })
    .filter((c) => c.entry) // skip IDs not found in raw journal (stale recall results)
    .sort((a, b) =>
      b.strategies - a.strategies ||
      b.confirmedCount - a.confirmedCount ||
      b.id - a.id, // higher id = more recent
    )
    .slice(0, 10);

  return candidates.map((c) => c.entry);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Run all four retrieval strategies in parallel and return a fused, ranked
 * list of up to 10 journal entries as a context string for the LLM.
 */
export const hybridJournalRecall = _G.hybridJournalRecallLib = async (spawn, memoDbPath, emailText, fingerprint, envelope) => {
  return _G.traceStep('🔎', 'Hybrid journal recall', async () => {
    const allEntries = await readJournalEntries(memoDbPath);
    if (!allEntries.length) {
      return { context: 'No relevant journal entry found.', entries: [] };
    }

    const s1 = await recallBySender(spawn, memoDbPath, envelope?.senderEmail);
    const s2 = await recallByKeywords(spawn, memoDbPath, fingerprint, emailText);
    const [s3, s4] = await Promise.all([
      recallByQA(spawn, memoDbPath, fingerprint),
      recallByVector(spawn, memoDbPath, emailText),
    ]);

    _G.log('recall.strategies', {
      s1: s1.map((r) => r.id),
      s2: s2.map((r) => r.id),
      s3: s3.map((r) => r.id),
      s4: s4.map((r) => r.id),
    }, 'memo');

    const fused = fuseResults(allEntries, s1, s2, s3, s4);

    const context = fused.length
      ? `[${fused.map(formatEntryAsContext).join(',\n')}]`
      : 'No relevant journal entry found.';

    return { context, entries: fused };
  });
};

/**
 * Read every journal entry from the raw YAML file and return them as an array.
 * Exported so agent.mjs can use it for the re-index relevance filter loop.
 */
export const readAllJournalEntries = _G.readAllJournalEntriesLib = async (memoDbPath) => {
  return readJournalEntries(memoDbPath);
};

/** Format a single raw journal entry as a text block for LLM input. */
export const formatJournalEntryForPrompt = _G.formatJournalEntryForPromptLib = (entry) => {
  return `[Journal ${entry.id}]\n${entry.body.trim()}`;
};
