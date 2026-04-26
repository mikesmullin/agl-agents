import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { resolve } from 'path';
import { _G } from './globals.mjs';

const CONFIG_PATH = resolve(process.cwd(), 'config.yaml');

function parseGoogleEmailYaml(yamlText) {
  try {
    return YAML.parse(_G.mustBeStringOr(yamlText, '')) || {};
  }
  catch (err) {
    throw new Error(`Failed to parse google-email YAML output: ${err?.message || String(err)}`);
  }
}

export const listEmailIds = _G.listEmailIds = (listOutputYaml) => {
  const doc = parseGoogleEmailYaml(listOutputYaml);
  const emails = Array.isArray(doc?.emails) ? doc.emails : [];

  return emails
    .map((email) => email?.shortId || email?.id)
    .filter(Boolean)
    .map(String);
};

export const hasPendingMutations = _G.hasPendingMutations = (planOutputYaml) => {
  const doc = parseGoogleEmailYaml(planOutputYaml);
  const pendingEmails = Number(doc?.pendingEmails || 0);
  const totalActions = Number(doc?.totalActions || 0);
  const pendingList = Array.isArray(doc?.pending) ? doc.pending : [];
  return pendingEmails > 0 || totalActions > 0 || pendingList.length > 0;
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
    const labels = config?.google_email?.labels;
    return labels && typeof labels === 'object' && !Array.isArray(labels) ? labels : {};
  }
  catch {
    return {};
  }
}

function folderName(folder) {
  return _G.mustBeTrimmedStringOr(folder?.name, '');
}

export const renderMoveFolderChoices = _G.renderMoveFolderChoicesLib = (folders = [], { includeArchive = true, fallback = '(none loaded)' } = {}) => {
  if (!Array.isArray(folders) || folders.length === 0) {
    return includeArchive ? 'archive: remove from inbox without moving to a named folder' : String(fallback);
  }

  const lines = folders.map((folder) => {
    const name = folderName(folder);
    const purpose = _G.mustBeTrimmedStringOr(folder?.purpose, '');
    return purpose ? `- ${name}: ${purpose}` : `- ${name}:`;
  });

  if (includeArchive) {
    lines.push('archive: remove from inbox without moving to a named folder');
  }

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
  return _G.traceStep('🗃️', 'Loading Gmail folder cache', async () => {
    const labels = await _G.spawn('google-email', ['labels', '--yaml'], { assertExit0: true });

    let parsed = {};
    try {
      parsed = normalizeDoc(YAML.parse(_G.mustBeStringOr(labels.stdout, '')));
    }
    catch {
      _G.cachedMoveFolders = [];
      return _G.cachedMoveFolders;
    }

    const labelRows = Array.isArray(parsed?.labels) ? parsed.labels : [];
    const purposeConfig = await loadFolderPurposeConfig();
    const folderNames = [...new Set(
      labelRows
        .filter((label) => String(label?.type || '').toLowerCase() === 'user')
        .map((label) => _G.mustBeTrimmedStringOr(label?.name, ''))
        .filter(Boolean),
    )].sort((a, b) => a.localeCompare(b));

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
    'google-email',
    ['inbox', 'view', emailId, '--yaml'],
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
  const filterTrace = _G.traceStart('🧽', 'Pre-filtering email markup');
  let prefilteredBody;
  try {
    prefilteredBody = await _G.prefilterEmailForSummary(emailText);
    filterTrace.traceEnd();
  }
  catch (err) {
    filterTrace.traceFail();
    throw err;
  }

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
      'google-email',
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
    const list = await spawn('google-email', ['inbox', 'list', '--limit', String(limit), '--yaml']);
    if (list.code !== 0) {
      throw new Error(`google-email inbox list failed:\n${list.stderr || list.stdout}`);
    }
    return listEmailIds(list.stdout);
  });
};

_G.endEmailTransaction = async () => {
  const planTrace = _G.traceStart('🧾', 'Planning queued mutations');
  await _G.spawn('google-email', ['plan'], { assertExit0: true, stdio: 'inherit' });
  planTrace.traceEnd();

  const applyTrace = _G.traceStart('🚀', 'Applying queued mutations');
  await _G.spawn('google-email', ['apply'], { assertExit0: true, stdio: 'inherit' });
  applyTrace.traceEnd();

  const cleanTrace = _G.traceStart('🧹', 'Cleaning local google-email cache');
  await _G.spawn('google-email', ['clean'], { assertExit0: true, stdio: 'inherit' });
  cleanTrace.traceEnd();
};


