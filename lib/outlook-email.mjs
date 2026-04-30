import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { resolve } from 'path';
import { _G } from './globals.mjs';

const CONFIG_PATH = resolve(process.cwd(), 'config.yaml');

function parseOutlookEmailYaml(yamlText) {
  try {
    return YAML.parse(_G.mustBeStringOr(yamlText, '')) || {};
  }
  catch (err) {
    throw new Error(`Failed to parse outlook-email YAML output: ${err?.message || String(err)}`);
  }
}

// Strip ANSI escape codes (colours, cursor-move sequences, etc.)
const stripAnsi = (str) => String(str || '').replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1B\[\d*G/g, '');

// Parse the human-readable list output:
//    1.  659520  01/05      Jane Smith    Subject line
export const listEmailIds = _G.listEmailIds = (listOutput) => {
  const lines = stripAnsi(listOutput).split('\n');
  const ids = [];
  const lineRe = /^\s+\d+\.\s+([a-f0-9]+)\s/;
  for (const line of lines) {
    const m = line.match(lineRe);
    if (m) ids.push(m[1]);
  }
  return ids;
};

export const hasPendingMutations = _G.hasPendingMutations = (planOutput) => {
  const text = String(planOutput || '');
  return !text.includes('No pending changes');
};

function normalizeDoc(parsed) {
  if (Array.isArray(parsed)) {
    const docs = parsed.filter((d) => d && typeof d === 'object' && !Array.isArray(d));
    if (docs.length === 0) return parsed[0] || {};
    const merged = {};
    for (const d of docs) {
      Object.assign(merged, d);
    }
    return merged;
  }
  if (parsed && typeof parsed === 'object') return parsed;
  return {};
}

async function loadFolderPurposeConfig() {
  try {
    const configText = await readFile(CONFIG_PATH, 'utf8');
    const config = YAML.parse(_G.mustBeStringOr(configText, '')) || {};
    const folders = config?.outlook_email?.folders;
    return folders && typeof folders === 'object' && !Array.isArray(folders) ? folders : {};
  }
  catch {
    return {};
  }
}

// Parse the ASCII tree output of `outlook-email folders`:
//   ├── Alerts  9637 unread, 12919 total
//   │   ├── SubFolder  (empty)
function parseOutlookFolderNames(treeOutput) {
  const names = [];
  const lineRe = /[├└]── (.+?)(?:\s{2,}|$)/;
  for (const line of stripAnsi(treeOutput).split('\n')) {
    const m = line.match(lineRe);
    if (m) {
      const name = m[1].trim();
      if (name) names.push(name);
    }
  }
  return names;
}

function folderName(folder) {
  return _G.mustBeTrimmedStringOr(folder?.name, '');
}

export const renderMoveFolderChoices = _G.renderMoveFolderChoicesLib = (folders = [], { includeArchive = true, fallback = '(none loaded)' } = {}) => {
  if (!Array.isArray(folders) || folders.length === 0) {
    return includeArchive ? 'Archive: remove from inbox without moving to a named folder' : String(fallback);
  }

  const lines = folders.map((folder) => {
    const name = folderName(folder);
    const purpose = _G.mustBeTrimmedStringOr(folder?.purpose, '');
    return purpose ? `- ${name}: ${purpose}` : `- ${name}:`;
  });

  return lines.join('\n') || String(fallback);
};

export const findMoveFolder = _G.findMoveFolderLib = (folders = [], requestedFolder = '') => {
  const exact = _G.mustBeTrimmedStringOr(requestedFolder, '');
  return Array.isArray(folders) ? folders.find((folder) => folderName(folder) === exact) || null : null;
};

function firstRecipientAddress(emailDoc) {
  const to = Array.isArray(emailDoc?.toRecipients) ? emailDoc.toRecipients : [];
  const first = to[0] || {};
  return _G.mustBeTrimmedStringOr(first?.address || first?.emailAddress?.address, '');
}

function senderText(emailDoc) {
  const from = emailDoc?.from || emailDoc?.sender || {};
  const name = _G.mustBeTrimmedStringOr(from?.name || from?.emailAddress?.name, '');
  const address = _G.mustBeTrimmedStringOr(from?.address || from?.emailAddress?.address, '');
  if (name && address) return `${name} <${address}>`;
  return name || address || 'Unknown sender';
}

export const loadMoveFolderCache = _G.loadMoveFolderCacheLib = async () => {
  return _G.traceStep('🗃️', 'Loading Outlook folder cache', async () => {
    const result = await _G.spawn('outlook-email', ['folders'], { assertExit0: true });

    const folderNames = [...new Set(parseOutlookFolderNames(result.stdout))].sort((a, b) =>
      a.localeCompare(b),
    );

    const purposeConfig = await loadFolderPurposeConfig();

    _G.cachedMoveFolders = folderNames.map((name) => ({
      name,
      purpose: _G.mustBeTrimmedStringOr(purposeConfig?.[name], ''),
    }));

    _G.log('folders.cache.loaded', {
      count: _G.cachedMoveFolders.length,
      describedCount: _G.cachedMoveFolders.filter((folder) => folder.purpose).length,
    });
    return _G.cachedMoveFolders;
  });
};

export const invalidFolderMessage = _G.invalidFolderMessageLib = (requestedFolder, folders = []) => {
  const exact = _G.mustBeTrimmedStringOr(requestedFolder, '');
  const lower = exact.toLowerCase();
  const caseHint = folders.find((folder) => folderName(folder).toLowerCase() === lower)?.name || null;
  const sample = renderMoveFolderChoices(folders.slice(0, 20));
  const hint = caseHint ? ` Did you mean "${caseHint}"?` : '';
  return `Invalid folder "${exact}".${hint} Folder names are case-sensitive. Available folders:\n${sample}`;
};

export const extractEmailEnvelope = _G.extractEmailEnvelope = (emailYamlText) => {
  try {
    const parsed = YAML.parse(String(emailYamlText || ''));
    const doc = normalizeDoc(parsed);
    return {
      from: senderText(doc),
      to: firstRecipientAddress(doc) || 'Unknown recipient',
      subject: _G.mustBeStringOr(doc?.subject, '(no subject)'),
      date: _G.mustBeStringOr(doc?.receivedDateTime || doc?.sentDateTime, 'Unknown date'),
    };
  }
  catch {
    return {
      from: 'Unknown sender',
      to: 'Unknown recipient',
      subject: '(unavailable)',
      date: 'Unknown date',
    };
  }
};

export const buildDecisionEmailText = _G.buildDecisionEmailText = (envelope, prefilteredBody) => {
  const e = envelope || {};
  const header = [
    `To: ${_G.mustBeStringOr(e.to, 'Unknown recipient')}`,
    `From: ${_G.mustBeStringOr(e.from, 'Unknown sender')}`,
    `Subj: ${_G.mustBeStringOr(e.subject, '(no subject)')}`,
    `Date: ${_G.mustBeStringOr(e.date, 'Unknown date')}`,
  ].join('\n');

  const body = _G.mustBeTrimmedStringOr(prefilteredBody, '');
  if (!body) {
    return header;
  }
  return `${header}\n\n${body}`;
};

export const loadDecisionEmail = _G.loadDecisionEmail = async (emailId) => {
  const loadTrace = _G.traceStart('📨', `Loading email ${emailId}`);
  const view = _G.spawn(
    'outlook-email',
    ['view', emailId, '--yaml'],
    { assertExit0: true, scope: 'agent' },
  );
  try {
    await view.promise;
    loadTrace.traceEnd();
  }
  catch (err) {
    loadTrace.traceFail();
    throw err;
  }

  const emailText = view.stdout.trim();
  const prefilteredBody = await _G.prefilterEmailForSummary(emailText);
  const envelope = extractEmailEnvelope(emailText);
  const summaryInput = buildDecisionEmailText(envelope, prefilteredBody);
  return { emailText, envelope, prefilteredBody, summaryInput };
};

export const pullBatch = _G.pullBatchLib = async (spawn, { limit = 10, since = '14 days ago', log, traceEmoji = '📥', traceLabel = 'Pulling latest emails' } = {}) => {
  return _G.traceStep(traceEmoji, traceLabel, async () => {
    if (typeof log === 'function') {
      log('loop.pull.begin', { since, limit });
    }
    await spawn(
      'outlook-email',
      ['pull', '--since', since, '--limit', String(limit)],
      { assertExit0: true },
    );
    if (typeof log === 'function') {
      log('loop.pull.done');
    }
  });
};

export const loadPageIds = _G.loadPageIdsLib = async (spawn, { limit = 10, traceEmoji = '📋', traceLabel = 'Loading unread inbox page' } = {}) => {
  return _G.traceStep(traceEmoji, traceLabel, async () => {
    const list = await spawn('outlook-email', ['list', '--limit', String(limit)]);
    if (list.code !== 0) {
      throw new Error(`outlook-email list failed:\n${list.stderr || list.stdout}`);
    }
    return listEmailIds(list.stdout);
  });
};

_G.endEmailTransaction = async () => {
  const planTrace = _G.traceStart('🧾', 'Planning queued mutations');
  await _G.spawn('outlook-email', ['plan'], { assertExit0: true, stdio: 'inherit' });
  planTrace.traceEnd();

  const applyTrace = _G.traceStart('🚀', 'Applying queued mutations');
  await _G.spawn('outlook-email', ['apply', '--yes'], { assertExit0: true, stdio: 'inherit' });
  applyTrace.traceEnd();

  const cleanTrace = _G.traceStart('🧹', 'Cleaning local outlook-email cache');
  await _G.spawn('outlook-email', ['clean'], { assertExit0: true, stdio: 'inherit' });
  cleanTrace.traceEnd();
};

// --- Email provider adapter interface ---

_G.EMAIL_PROVIDER_NAME = 'Outlook';

_G.emailRead = async (emailId) => _G.spawn('outlook-email', ['read', emailId]);

_G.emailUnread = async (emailId) => _G.spawn('outlook-email', ['unread', emailId]);

_G.emailDelete = async (emailId) => _G.spawn('outlook-email', ['delete', emailId]);

_G.emailMove = async (emailId, folder) => {
  if (!_G.findMoveFolderLib(_G.cachedMoveFolders, folder)) {
    return { code: 1, stdout: '', stderr: _G.invalidFolderMessageLib(folder, _G.cachedMoveFolders) };
  }
  return _G.spawn('outlook-email', ['move', emailId, '--folder', folder]);
};
