import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { resolve } from 'path'
import { _G } from './globals.mjs'

CONFIG_PATH = resolve(process.cwd(), 'config.yaml')

parseGoogleEmailYaml = (yamlText) ->
  try
    YAML.parse(_G.mustBeStringOr(yamlText, '')) or {}
  catch err
    throw new Error("Failed to parse google-email YAML output: #{err?.message or String(err)}")

export listEmailIds = _G.listEmailIds = (listOutputYaml) ->
  doc = parseGoogleEmailYaml(listOutputYaml)
  emails = if Array.isArray(doc?.emails) then doc.emails else []

  emails
    .map((email) -> email?.shortId or email?.id)
    .filter(Boolean)
    .map(String)

export hasPendingMutations = _G.hasPendingMutations = (planOutputYaml) ->
  doc = parseGoogleEmailYaml(planOutputYaml)
  pendingEmails = Number(doc?.pendingEmails or 0)
  totalActions = Number(doc?.totalActions or 0)
  pendingList = if Array.isArray(doc?.pending) then doc.pending else []
  pendingEmails > 0 or totalActions > 0 or pendingList.length > 0

normalizeDoc = (parsed) ->
  if Array.isArray(parsed)
    docs = parsed.filter((d) -> d and typeof d === 'object' and not Array.isArray(d))
    if docs.length === 0 then return parsed[0] or {}
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
    labels = config?.google_email?.labels
    if labels and typeof labels === 'object' and not Array.isArray(labels) then labels else {}
  catch
    {}

folderName = (folder) ->
  _G.mustBeTrimmedStringOr(folder?.name, '')

export renderMoveFolderChoices = _G.renderMoveFolderChoicesLib = (folders = [], { includeArchive = true, fallback = '(none loaded)' } = {}) ->
  if not Array.isArray(folders) or folders.length === 0
    return if includeArchive then 'archive: remove from inbox without moving to a named folder' else String(fallback)

  lines = folders.map((folder) ->
    name = folderName(folder)
    purpose = _G.mustBeTrimmedStringOr(folder?.purpose, '')
    if purpose then "- #{name}: #{purpose}" else "- #{name}:"
  )

  if includeArchive
    lines.push('archive: remove from inbox without moving to a named folder')

  lines.join('\n') or String(fallback)

export findMoveFolder = _G.findMoveFolderLib = (folders = [], requestedFolder = '') ->
  exact = _G.mustBeTrimmedStringOr(requestedFolder, '')
  if Array.isArray(folders) then folders.find((folder) -> folderName(folder) === exact) ? null else null

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
  _G.traceStep('🗃️', 'Loading Gmail folder cache', ->
    labels = await _G.spawn('google-email', ['labels', '--yaml'], { assertExit0: true })

    parsed = {}
    try
      parsed = normalizeDoc(YAML.parse(_G.mustBeStringOr(labels.stdout, '')))
    catch
      _G.cachedMoveFolders = []
      return _G.cachedMoveFolders

    labelRows = if Array.isArray(parsed?.labels) then parsed.labels else []
    purposeConfig = await loadFolderPurposeConfig()
    folderNames = [...new Set(
      labelRows
        .filter((label) -> String(label?.type or '').toLowerCase() === 'user')
        .map((label) -> _G.mustBeTrimmedStringOr(label?.name, ''))
        .filter(Boolean),
    )].sort((a, b) -> a.localeCompare(b))

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
  caseHint = folders.find((folder) -> folderName(folder).toLowerCase() === lower)?.name ? null
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
    'google-email',
    ['inbox', 'view', emailId, '--all', '--yaml'],
    { assertExit0: true, scope: 'agent' },
  )
  try
    await view.promise
    loadTrace.traceEnd()
  catch err
    loadTrace.traceFail()
    throw err

  emailText = view.stdout.trim()
  filterTrace = _G.traceStart('🧽', 'Pre-filtering email markup')
  prefilteredBody = undefined
  try
    prefilteredBody = await _G.prefilterEmailForSummary(emailText)
    filterTrace.traceEnd()
  catch err
    filterTrace.traceFail()
    throw err

  envelope = extractEmailEnvelope(emailText)
  summaryInput = buildDecisionEmailText(envelope, prefilteredBody)
  { emailText, envelope, prefilteredBody, summaryInput }

export pullBatch = _G.pullBatchLib = (spawn, { limit = 10, since = '14 days ago', log, traceEmoji = '📥', traceLabel = 'Pulling latest emails' } = {}) ->
  _G.traceStep(traceEmoji, traceLabel, ->
    if typeof log === 'function'
      log('loop.pull.begin', { since, limit })
    await spawn(
      'google-email',
      ['pull', '--since', since, '--limit', String(limit)],
      { assertExit0: true },
    )
    if typeof log === 'function'
      log('loop.pull.done')
  )

export loadPageIds = _G.loadPageIdsLib = (spawn, { limit = 10, traceEmoji = '📋', traceLabel = 'Loading unread inbox page' } = {}) ->
  _G.traceStep(traceEmoji, traceLabel, ->
    list = await spawn('google-email', ['inbox', 'list', '--limit', String(limit), '--yaml'])
    if list.code !== 0
      throw new Error("google-email inbox list failed:\n#{list.stderr or list.stdout}")
    listEmailIds(list.stdout)
  )

_G.endEmailTransaction = ->
  planTrace = _G.traceStart('🧾', 'Planning queued mutations')
  await _G.spawn('google-email', ['plan'], { assertExit0: true, stdio: 'inherit' })
  planTrace.traceEnd()

  applyTrace = _G.traceStart('🚀', 'Applying queued mutations')
  await _G.spawn('google-email', ['apply'], { assertExit0: true, stdio: 'inherit' })
  applyTrace.traceEnd()

  cleanTrace = _G.traceStart('🧹', 'Cleaning local google-email cache')
  await _G.spawn('google-email', ['clean'], { assertExit0: true, stdio: 'inherit' })
  cleanTrace.traceEnd()

# --- Email provider adapter interface ---

_G.EMAIL_PROVIDER_NAME = 'Gmail'

_G.emailRead = (emailId) -> _G.spawn('google-email', ['inbox', 'read', emailId])

_G.emailUnread = (emailId) -> _G.spawn('google-email', ['inbox', 'unread', emailId])

_G.emailDelete = (emailId) -> _G.spawn('google-email', ['delete', emailId])

_G.emailMove = (emailId, folder) ->
  if _G.stristr(folder, 'archive')
    return _G.spawn('google-email', ['archive', emailId])
  if not _G.findMoveFolderLib(_G.cachedMoveFolders, folder)
    return { code: 1, stdout: '', stderr: _G.invalidFolderMessageLib(folder, _G.cachedMoveFolders) }
  _G.spawn('google-email', ['move', emailId, folder])
