import { YAML } from 'bun';
import { mkdir, writeFile, rm } from 'fs/promises';
import { resolve } from 'path';
import { _G } from './globals.mjs';

export const recallJournal = _G.recallJournal = async (spawn, memoDbPath, emailText) => {
  const isPresentationMemo = memoDbPath === _G.PRESENTATION_MEMO_DB;
  const topK = 10;
  return _G.traceStep(
    isPresentationMemo ? '🎛️' : '🔎',
    isPresentationMemo ? 'Searching presentation memo context' : 'Searching memo journal context',
    async () => {
      const query = emailText.slice(0, 1500);
      _G.log('memo.recall.begin', {
        dbPath: memoDbPath,
        isPresentationMemo,
        query,
        queryLength: query.length,
        originalLength: String(emailText || '').length,
        topK,
      }, 'memo');

      const result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', String(topK), '--yaml', query]);
      const output = result.code === 0 ? result.stdout.trim() : '';

      _G.log('memo.recall.result', {
        dbPath: memoDbPath,
        isPresentationMemo,
        code: result.code,
        stdout: result.stdout,
        stderr: result.stderr,
        output,
        outputLength: output.length,
      }, 'memo');

      if (result.code !== 0) {
        return '';
      }

      return output;
    },
  );
};

export const yamlBlock = _G.yamlBlock = (text) => {
  return String(text || '')
    .split('\n')
    .map((line) => `  ${line}`)
    .join('\n');
};

export const overwriteMemoById = _G.overwriteMemoByIdLib = async (spawn, dbDir, dbPath, memoId, bodyText) => {
  const id = Number(memoId);
  if (!Number.isInteger(id) || id < 0) {
    return { ok: false, message: `Invalid memo id: ${memoId}` };
  }

  await mkdir(dbDir, { recursive: true });
  const tmpFile = resolve(dbDir, `memo-edit-${id}-${Date.now()}.yaml`);
  const now = new Date().toISOString();
  const yamlDoc = [
    '---',
    `id: ${id}`,
    'metadata:',
    `  ts: "${now}"`,
    'body: |',
    yamlBlock(bodyText),
    '',
  ].join('\n');

  await writeFile(tmpFile, yamlDoc, 'utf8');
  const save = await spawn('memo', ['save', '-f', dbPath, tmpFile]);
  await rm(tmpFile, { force: true });

  if (save.code !== 0) {
    return { ok: false, message: save.stderr || save.stdout || 'Failed to save memo update.' };
  }
  return { ok: true, message: save.stdout || `Updated memo ${id}.` };
};

export const deleteMemoById = _G.deleteMemoByIdLib = async (spawn, dbDir, dbPath, memoId, reason = '') => {
  const note = [
    'deleted: true',
    `memo_id: ${memoId}`,
    reason ? `reason: ${String(reason).replaceAll('\n', ' ')}` : 'reason: user requested deletion',
  ].join('\n');
  return overwriteMemoById(spawn, dbDir, dbPath, memoId, note);
};

export const reindexMemoDb = _G.reindexMemoDbLib = async (spawn, dbPath) => {
  let reindex = await spawn('memo', ['reindex', '-f', dbPath]);
  if (reindex.code !== 0) {
    reindex = await spawn('memo', ['-f', dbPath, 'reindex']);
  }
  return reindex;
};

export const resolveMemoDbTarget = _G.resolveMemoDbTargetLib = (dbName, memoDb, presentationMemoDb) => {
  const db = String(dbName || '').trim().toLowerCase();
  if (db === 'journal') {
    return { ok: true, db, path: memoDb };
  }
  if (db === 'presentation') {
    return { ok: true, db, path: presentationMemoDb };
  }
  return {
    ok: false,
    message: 'Invalid db value. Use one of: journal, presentation.',
  };
};

export const saveJournalEntry = _G.saveJournalEntry = async (spawn, dbDir, memoDbPath, journalEntry) => {
  if (!journalEntry) return;

  await _G.traceStep('💾', 'Saving journal to memo', async () => {
    await mkdir(dbDir, { recursive: true });
    const tmpFile = resolve(dbDir, `memo-save-${Date.now()}.yaml`);
    const now = new Date().toISOString();

    const yamlDoc = [
      '---',
      'metadata:',
      `  ts: "${now}"`,
      'body: |',
      `  summary: ${String(journalEntry.summary || '').replaceAll('\n', ' ')}`,
      `  keywords: ${String(journalEntry.keywords || '').replaceAll('\n', ' ')}`,
      `  action_taken: ${String(journalEntry.action_taken || '').replaceAll('\n', ' ')}`,
      `  factors: ${String(journalEntry.factors || '').replaceAll('\n', ' ')}`,
      '',
    ].join('\n');

    await writeFile(tmpFile, yamlDoc, 'utf8');
    const save = await spawn('memo', ['save', '-f', memoDbPath, tmpFile]);
    await rm(tmpFile, { force: true });

    if (save.code !== 0) {
      console.error('Failed to save journal entry to memo.');
      if (save.stderr) console.error(save.stderr.trim());
    }
  });
};

export const savePresentationEntry = _G.savePresentationEntry = async (spawn, dbDir, presentationMemoDbPath, presentationEntry) => {
  if (!presentationEntry) return;

  await _G.traceStep('📚', 'Saving presentation memo', async () => {
    await mkdir(dbDir, { recursive: true });
    const tmpFile = resolve(dbDir, `presentation-save-${Date.now()}.yaml`);
    const body = YAML.stringify({
      applies_if: String(presentationEntry.applies_if || '').replaceAll('\n', ' '),
      formatting_instructions: String(presentationEntry.formatting_instructions || '').replaceAll('\n', ' '),
    }).trimEnd();
    const yamlDoc = YAML.stringify({
      metadata: { ts: new Date().toISOString() },
      body,
    });
    await writeFile(tmpFile, yamlDoc, 'utf8');
    const save = await spawn('memo', ['save', '-f', presentationMemoDbPath, tmpFile]);
    await rm(tmpFile, { force: true });

    if (save.code !== 0) {
      console.error('Failed to save presentation entry to memo.');
      if (save.stderr) console.error(save.stderr.trim());
    }
  });
};

export const parseMemoRecallResults = _G.parseMemoRecallResults = (text) => {
  const src = String(text || '').trim();
  if (!src) {
    return [];
  }

  try {
    const parsed = YAML.parse(src) || {};
    const rows = Array.isArray(parsed?.results) ? parsed.results : [];
    return rows.map((row, index) => ({
      id: Number(row?.id || 0),
      rank: index + 1,
      score: Number(row?.score || 0),
      content: String(row?.body || '').trim(),
    }));
  }
  catch {
    const results = [];
    const re = /(?:^|\n)\s*\[(\d+)\]\s+Score:\s*([0-9.]+)\s*\|\s*\n([\s\S]*?)(?=(?:\n\s*\[\d+\]\s+Score:)|$)/g;
    let m;
    while ((m = re.exec(src)) !== null) {
      const rank = Number(m[1]);
      const score = Number(m[2]);
      const content = String(m[3] || '')
        .split('\n')
        .map((line) => line.replace(/^\s{2,}/, ''))
        .join('\n')
        .trim();
      results.push({ id: 0, rank, score, content });
    }
    return results;
  }
};

export const parseMemoBodyFields = _G.parseMemoBodyFields = (text) => {
  const lines = String(text || '').split('\n');
  const fields = {};
  let currentKey = '';

  for (const rawLine of lines) {
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
};

export const buildJournalContext = _G.buildJournalContext = (journalRecallText) => {
  const rows = parseMemoRecallResults(journalRecallText);
  if (!rows.length) {
    return 'No relevant journal entry found.';
  }

  const payload = rows.map((row) => {
    const fields = parseMemoBodyFields(row.content);
    return {
      id: row.id,
      rank: row.rank,
      score: row.score,
      summary: String(fields.summary || ''),
      keywords: String(fields.keywords || ''),
      action_taken: String(fields.action_taken || ''),
      factors: String(fields.factors || ''),
      raw_excerpt: row.content.slice(0, 1200),
    };
  });

  return JSON.stringify(payload, null, 2);
};

export const extractPresentationCandidateFromRecall = _G.extractPresentationCandidateFromRecall = (presentationRecallText) => {
  const rows = parseMemoRecallResults(presentationRecallText);
  if (!rows.length) {
    return {
      has_formatting_instructions: false,
      applies_if: '',
      formatting_instructions: '',
    };
  }

  const top = rows[0];
  const fields = parseMemoBodyFields(top.content);
  const appliesIf = String(fields.applies_if || '').trim();
  const formattingInstructions = String(fields.formatting_instructions || '').trim();

  return {
    has_formatting_instructions: Boolean(formattingInstructions),
    applies_if: appliesIf,
    formatting_instructions: formattingInstructions,
  };
};

export const extractPresentationPreferences = _G.extractPresentationPreferences = async (emailText, journalMatches) => {
  return _G.traceStep('🎯', 'Extracting presentation preferences', async () => {
    void emailText;
    return extractPresentationCandidateFromRecall(journalMatches);
  });
};
