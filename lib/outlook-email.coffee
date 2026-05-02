import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { resolve } from 'path'
import { _G } from './globals.mjs'

CONFIG_PATH = resolve(process.cwd(), 'config.yaml')

parseOutlookEmailYaml = (yamlText) ->
  try
    YAML.parse(_G.mustBeStringOr(yamlText, '')) or {}
  catch err
    throw new Error("Failed to parse outlook-email YAML output: #{err?.message or String(err)}")

# Strip ANSI escape codes (colours, cursor-move sequences, etc.)
stripAnsi = (str) -> String(str or '').replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1B\[\d*G/g, '')

# Parse the human-readable list output:
#    1.  659520  01/05      Jane Smith    Subject line
export listEmailIds = _G.listEmailIds = (listOutput) ->
  lines = stripAnsi(listOutput).split('\n')
  ids = []
  lineRe = /^\s+\d+\.\s+([a-f0-9]+)\s/
  for line in lines
    m = line.match(lineRe)
    if m then ids.push(m[1])
  ids

export hasPendingMutations = _G.hasPendingMutations = (planOutput) ->
  text = String(planOutput or '')
  not text.includes('No pending changes')

normalizeDoc = (parsed) ->
  if Array.isArray(parsed)
    docs = parsed.filter((d) -> d and typeof d === 'object' and not Array.isArray(d))
    if docs.length is 0 then return parsed[0] or {}
    merged = {}
    for d in docs
      Object.assign(merged, d)
    return merged
  if parsed and typeof parsed === 'object' then return parsed
  {}

loadFolderPurposeConfig = ->
  try
    configText = await readFile(CONFIG_PATH, 'utf8')
    config = YAML.parse(_G.mustBeStringOr(configText, '')) or {}
    folders = config?.outlook_email?.folders
    if folders and typeof folders === 'object' and not Array.isArray(folders) then folders else {}
  catch
    {}

# Parse the ASCII tree output of `outlook-email folders`:
#   ├── Alerts  9637 unread, 12919 total
#   │   ├── SubFolder  (empty)
parseOutlookFolderNames = (treeOutput) ->
  names = []
  lineRe = /[├└]── (.+?)(?:\s{2,}|$)/
  for line in stripAnsi(treeOutput).split('\n')
    m = line.match(lineRe)
    if m
      name = m[1].trim()
      if name then names.push(name)
  names

folderName = (folder) ->
  _G.mustBeTrimmedStringOr(folder?.name, '')

export renderMoveFolderChoices = _G.renderMoveFolderChoicesLib = (folders = [], { includeArchive = true, fallback = '(none loaded)' } = {}) ->
  if not Array.isArray(folders) or folders.length is 0
    return if includeArchive then 'Archive: remove from inbox without moving to a named folder' else String(fallback)

  lines = folders.map((folder) ->
    name = folderName(folder)
    purpose = _G.mustBeTrimmedStringOr(folder?.purpose, '')
    if purpose then "- #{name}: #{purpose}" else "- #{name}:"
  )

  lines.join('\n') or String(fallback)

export findMoveFolder = _G.findMoveFolderLib = (folders = [], requestedFolder = '') ->
  exact = _G.mustBeTrimmedStringOr(requestedFolder, '')
  if Array.isArray(folders) then folders.find((folder) -> folderName(folder) is exact) ? null else null

firstRecipientAddress = (emailDoc) ->
  to = if Array.isArray(emailDoc?.toRecipients) then emailDoc.toRecipients else []
  first = to[0] or {}
  _G.mustBeTrimmedStringOr(first?.address or first?.emailAddress?.address, '')

senderText = (emailDoc) ->
  from = emailDoc?.from or emailDoc?.sender or {}
  name = _G.mustBeTrimmedStringOr(from?.name or from?.emailAddress?.name, '')
  address = _G.mustBeTrimmedStringOr(from?.address or from?.emailAddress?.address, '')
  if name and address then return "#{name} <#{address}>"
  name or address or 'Unknown sender'

export loadMoveFolderCache = _G.loadMoveFolderCacheLib = ->
  _G.traceStep('🗃️', 'Loading Outlook folder cache', ->
    result = await _G.spawn('outlook-email', ['folders'], { assertExit0: true })

    folderNames = [...new Set(parseOutlookFolderNames(result.stdout))].sort((a, b) ->
      a.localeCompare(b)
    )

    purposeConfig = await loadFolderPurposeConfig()

    _G.cachedMoveFolders = folderNames.map((name) ->
      name: name
      purpose: _G.mustBeTrimmedStringOr(purposeConfig?.[name], '')
    )

    _G.log('folders.cache.loaded',
      count: _G.cachedMoveFolders.length
      describedCount: _G.cachedMoveFolders.filter((folder) -> folder.purpose).length
    )
    _G.cachedMoveFolders
  )

export invalidFolderMessage = _G.invalidFolderMessageLib = (requestedFolder, folders = []) ->
  exact = _G.mustBeTrimmedStringOr(requestedFolder, '')
  lower = exact.toLowerCase()
  caseHint = folders.find((folder) -> folderName(folder).toLowerCase() is lower)?.name ? null
  sample = renderMoveFolderChoices(folders.slice(0, 20))
  hint = if caseHint then " Did you mean \"#{caseHint}\"?" else ''
  "Invalid folder \"#{exact}\".#{hint} Folder names are case-sensitive. Available folders:\n#{sample}"

export extractEmailEnvelope = _G.extractEmailEnvelope = (emailYamlText) ->
  tryParse = (text) ->
    try
      parsed = YAML.parse(String(text or '').replace(/[^\x00-\x7F]/g, ''))
      doc = normalizeDoc(parsed)
      from: senderText(doc)
      to: firstRecipientAddress(doc) or 'Unknown recipient'
      subject: _G.mustBeStringOr(doc?.subject, '(no subject)')
      date: _G.mustBeStringOr(doc?.receivedDateTime or doc?.sentDateTime, 'Unknown date')
    catch then return null

  # First attempt: parse the full YAML.
  result = tryParse(emailYamlText)
  if result then return result

  # Second attempt: the body HTML often contains characters that confuse YAML.parse.
  # Envelope fields (from, subject, receivedDateTime) are always before body:, so strip it.
  stripped = String(emailYamlText or '').replace(/^body:[\s\S]*/m, '')
  result2 = tryParse(stripped)
  if result2 then return result2

  from: 'Unknown sender'
  to: 'Unknown recipient'
  subject: '(unavailable)'
  date: 'Unknown date'

export buildDecisionEmailText = _G.buildDecisionEmailText = (envelope, prefilteredBody) ->
  e = envelope or {}
  header = [
    "To: #{_G.mustBeStringOr(e.to, 'Unknown recipient')}"
    "From: #{_G.mustBeStringOr(e.from, 'Unknown sender')}"
    "Subj: #{_G.mustBeStringOr(e.subject, '(no subject)')}"
    "Date: #{_G.mustBeStringOr(e.date, 'Unknown date')}"
  ].join('\n')

  body = _G.mustBeTrimmedStringOr(prefilteredBody, '')
  if not body
    return header
  "#{header}\n\n#{body}"

export loadDecisionEmail = _G.loadDecisionEmail = (emailId) ->
  loadTrace = _G.traceStart('📨', "Loading email #{emailId}")
  view = _G.spawn(
    'outlook-email'
    ['view', emailId, '--yaml']
    { assertExit0: true, scope: 'agent' }
  )
  try
    await view.promise
    loadTrace.traceEnd()
  catch err
    loadTrace.traceFail()
    throw err

  emailText = view.stdout.trim()
  prefilteredBody = await _G.prefilterEmailForSummary(emailText)
  envelope = extractEmailEnvelope(emailText)
  summaryInput = buildDecisionEmailText(envelope, prefilteredBody)
  { emailText, envelope, prefilteredBody, summaryInput }

export pullBatch = _G.pullBatchLib = (spawn, { limit = 10, since = '14 days ago', log, traceEmoji = '📥', traceLabel = 'Pulling latest emails' } = {}) ->
  _G.traceStep(traceEmoji, traceLabel, ->
    if typeof log is 'function'
      log('loop.pull.begin', { since, limit })
    await spawn(
      'outlook-email'
      ['pull', '--since', since, '--limit', String(limit)]
      { assertExit0: true }
    )
    if typeof log is 'function'
      log('loop.pull.done')
  )

export loadPageIds = _G.loadPageIdsLib = (spawn, { limit = 10, traceEmoji = '📋', traceLabel = 'Loading unread inbox page' } = {}) ->
  _G.traceStep(traceEmoji, traceLabel, ->
    list = await spawn('outlook-email', ['list', '--limit', String(limit)])
    if list.code isnt 0
      throw new Error("outlook-email list failed:\n#{list.stderr or list.stdout}")
    listEmailIds(list.stdout)
  )

_G.endEmailTransaction = ->
  planTrace = _G.traceStart('🧾', 'Planning queued mutations')
  await _G.spawn('outlook-email', ['plan'], { assertExit0: true, stdio: 'inherit' })
  planTrace.traceEnd()

  applyTrace = _G.traceStart('🚀', 'Applying queued mutations')
  await _G.spawn('outlook-email', ['apply', '--yes'], { assertExit0: true, stdio: 'inherit' })
  applyTrace.traceEnd()

  cleanTrace = _G.traceStart('🧹', 'Cleaning local outlook-email cache')
  await _G.spawn('outlook-email', ['clean'], { assertExit0: true, stdio: 'inherit' })
  cleanTrace.traceEnd()

# --- Email provider adapter interface ---

_G.EMAIL_PROVIDER_NAME = 'Outlook'

_G.emailRead = (emailId) -> _G.spawn('outlook-email', ['read', emailId])

_G.emailUnread = (emailId) -> _G.spawn('outlook-email', ['unread', emailId])

_G.emailDelete = (emailId) -> _G.spawn('outlook-email', ['delete', emailId])

_G.emailMove = (emailId, folder) ->
  if not _G.findMoveFolderLib(_G.cachedMoveFolders, folder)
    return { code: 1, stdout: '', stderr: _G.invalidFolderMessageLib(folder, _G.cachedMoveFolders) }
  _G.spawn('outlook-email', ['move', emailId, '--folder', folder])
