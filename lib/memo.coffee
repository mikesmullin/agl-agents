import { YAML } from 'bun'
import { mkdir, writeFile, rm } from 'fs/promises'
import { resolve } from 'path'
import { _G } from './globals.mjs'

export recallJournal = _G.recallJournal = (spawn, memoDbPath, emailText) ->
  isPresentationMemo = memoDbPath === _G.PRESENTATION_MEMO_DB
  topK = 10
  _G.traceStep(
    if isPresentationMemo then '🎛️' else '🔎',
    if isPresentationMemo then 'Searching presentation memo context' else 'Searching memo journal context',
    () ->
      query = emailText.slice(0, 1500)
      _G.log 'memo.recall.begin',
        dbPath: memoDbPath
        isPresentationMemo
        query
        queryLength: query.length
        originalLength: String(emailText or '').length
        topK
      , 'memo'

      result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', String(topK), '--yaml', query])
      output = if result.code === 0 then result.stdout.trim() else ''

      _G.log 'memo.recall.result',
        dbPath: memoDbPath
        isPresentationMemo
        code: result.code
        stdout: result.stdout
        stderr: result.stderr
        output
        outputLength: output.length
      , 'memo'

      if result.code !== 0
        return ''

      output
  )

export yamlBlock = _G.yamlBlock = (text) ->
  String(text or '')
    .split('\n')
    .map((line) -> "  #{line}")
    .join('\n')

export overwriteMemoById = _G.overwriteMemoByIdLib = (spawn, dbDir, dbPath, memoId, bodyText) ->
  id = Number(memoId)
  if not Number.isInteger(id) or id < 0
    return { ok: false, message: "Invalid memo id: #{memoId}" }

  await mkdir(dbDir, { recursive: true })
  tmpFile = resolve(dbDir, "memo-edit-#{id}-#{Date.now()}.yaml")
  now = new Date().toISOString()
  yamlDoc = [
    '---'
    "id: #{id}"
    'metadata:'
    "  ts: \"#{now}\""
    'body: |'
    yamlBlock(bodyText)
    ''
  ].join('\n')

  await writeFile(tmpFile, yamlDoc, 'utf8')
  save = await spawn('memo', ['save', '-f', dbPath, tmpFile])
  await rm(tmpFile, { force: true })

  if save.code !== 0
    return { ok: false, message: save.stderr or save.stdout or 'Failed to save memo update.' }
  { ok: true, message: save.stdout or "Updated memo #{id}." }

export deleteMemoById = _G.deleteMemoByIdLib = (spawn, dbDir, dbPath, memoId, _reason = '') ->
  id = Number(memoId)
  if not Number.isInteger(id) or id < 0
    return { ok: false, message: "Invalid memo id: #{memoId}" }
  void dbDir
  del = await spawn('memo', ['delete', '-f', dbPath, String(id)])
  if del.code !== 0
    return { ok: false, message: del.stderr or del.stdout or "Failed to delete memo #{id}." }
  { ok: true, message: del.stdout or "Deleted memo #{id}." }

export reindexMemoDb = _G.reindexMemoDbLib = (spawn, dbPath) ->
  reindex = await spawn('memo', ['reindex', '-f', dbPath])
  if reindex.code !== 0
    reindex = await spawn('memo', ['-f', dbPath, 'reindex'])
  reindex

export resolveMemoDbTarget = _G.resolveMemoDbTargetLib = (dbName, memoDb, presentationMemoDb) ->
  db = String(dbName or '').trim().toLowerCase()
  if db === 'journal'
    return { ok: true, db, path: memoDb }
  if db === 'presentation'
    return { ok: true, db, path: presentationMemoDb }
  {
    ok: false
    message: 'Invalid db value. Use one of: journal, presentation.'
  }

export saveJournalEntry = _G.saveJournalEntry = (spawn, dbDir, memoDbPath, journalEntry) ->
  if not journalEntry then return

  await _G.traceStep '💾', 'Saving journal to memo', () ->
    await mkdir(dbDir, { recursive: true })
    tmpFile = resolve(dbDir, "memo-save-#{Date.now()}.yaml")
    now = new Date().toISOString()

    keywords = [...new Set(
      String(journalEntry.keywords or '')
        .split(',')
        .map((keyword) -> keyword.trim())
        .filter(Boolean)
    )]

    body = [
      "summary: #{String(journalEntry.summary or '').replaceAll('\n', ' ')}"
      "action_taken: #{String(journalEntry.action_taken or '').replaceAll('\n', ' ')}"
      "factors: #{String(journalEntry.factors or '').replaceAll('\n', ' ')}"
      "sender_offers: #{String(journalEntry.sender_offers or '').replaceAll('\n', ' ')}"
      "sender_expects: #{String(journalEntry.sender_expects or '').replaceAll('\n', ' ')}"
      "reader_value: #{String(journalEntry.reader_value or '').replaceAll('\n', ' ')}"
      "match_criteria: #{String(journalEntry.match_criteria or '').replaceAll('\n', ' ')}"
      "rule: #{String(journalEntry.rule or '').replaceAll('\n', ' ')}"
      "applies_if: #{String(journalEntry.applies_if or '').replaceAll('\n', ' ')}"
    ].join('\n')

    yamlDoc = YAML.stringify(
      metadata:
        ts: now
        confirmed_count: 0
        last_confirmed_ts: null
        sender_email: String(journalEntry.sender_email or '').trim().toLowerCase()
        keywords
      body
    )

    await writeFile(tmpFile, yamlDoc, 'utf8')
    save = await spawn('memo', ['save', '-f', memoDbPath, tmpFile])
    await rm(tmpFile, { force: true })

    if save.code !== 0
      console.error 'Failed to save journal entry to memo.'
      if save.stderr then console.error save.stderr.trim()

export savePresentationEntry = _G.savePresentationEntry = (spawn, dbDir, presentationMemoDbPath, presentationEntry) ->
  if not presentationEntry then return

  await _G.traceStep '📚', 'Saving presentation memo', () ->
    await mkdir(dbDir, { recursive: true })
    tmpFile = resolve(dbDir, "presentation-save-#{Date.now()}.yaml")
    body = YAML.stringify(
      applies_if: String(presentationEntry.applies_if or '').replaceAll('\n', ' ')
      formatting_instructions: String(presentationEntry.formatting_instructions or '').replaceAll('\n', ' ')
    ).trimEnd()
    yamlDoc = YAML.stringify(
      metadata: { ts: new Date().toISOString() }
      body
    )
    await writeFile(tmpFile, yamlDoc, 'utf8')
    save = await spawn('memo', ['save', '-f', presentationMemoDbPath, tmpFile])
    await rm(tmpFile, { force: true })

    if save.code !== 0
      console.error 'Failed to save presentation entry to memo.'
      if save.stderr then console.error save.stderr.trim()

export parseMemoRecallResults = _G.parseMemoRecallResults = (text) ->
  src = String(text or '').trim()
  if not src
    return []

  try
    parsed = YAML.parse(src) or {}
    rows = if Array.isArray(parsed?.results) then parsed.results else []
    rows.map((row, index) ->
      id: Number(row?.id or 0)
      rank: index + 1
      score: Number(row?.score or 0)
      content: String(row?.body or '').trim()
    )
  catch
    results = []
    re = /(?:^|\n)\s*\[(\d+)\]\s+Score:\s*([0-9.]+)\s*\|\s*\n([\s\S]*?)(?=(?:\n\s*\[\d+\]\s+Score:)|$)/g
    m = undefined
    while (m = re.exec(src)) !== null
      rank = Number(m[1])
      score = Number(m[2])
      content = String(m[3] or '')
        .split('\n')
        .map((line) -> line.replace(/^\s{2,}/, ''))
        .join('\n')
        .trim()
      results.push({ id: 0, rank, score, content })
    results

export parseMemoBodyFields = _G.parseMemoBodyFields = (text) ->
  lines = String(text or '').split('\n')
  fields = {}
  currentKey = ''

  for rawLine in lines
    line = rawLine.replace(/\r$/, '')
    kv = line.match(/^\s*([a-zA-Z_][a-zA-Z0-9_-]*):\s*(.*)$/)
    if kv
      currentKey = kv[1]
      fields[currentKey] = String(kv[2] or '').trim()
      continue

    if currentKey and /^\s+\S/.test(line)
      fields[currentKey] = "#{fields[currentKey]} #{line.trim()}".trim()

  fields

export buildJournalContext = _G.buildJournalContext = (journalRecallText) ->
  rows = parseMemoRecallResults(journalRecallText)
  if not rows.length
    return 'No relevant journal entry found.'

  payload = rows.map((row) ->
    fields = parseMemoBodyFields(row.content)
    {
      id: row.id
      rank: row.rank
      score: row.score
      summary: String(fields.summary or '')
      keywords: String(fields.keywords or '')
      action_taken: String(fields.action_taken or '')
      factors: String(fields.factors or '')
      raw_excerpt: row.content.slice(0, 1200)
    }
  )

  JSON.stringify(payload, null, 2)

export extractPresentationCandidateFromRecall = _G.extractPresentationCandidateFromRecall = (presentationRecallText) ->
  rows = parseMemoRecallResults(presentationRecallText)
  if not rows.length
    return {
      has_formatting_instructions: false
      applies_if: ''
      formatting_instructions: ''
    }

  top = rows[0]
  fields = parseMemoBodyFields(top.content)
  appliesIf = String(fields.applies_if or '').trim()
  formattingInstructions = String(fields.formatting_instructions or '').trim()

  {
    has_formatting_instructions: Boolean(formattingInstructions)
    applies_if: appliesIf
    formatting_instructions: formattingInstructions
  }

export extractPresentationPreferences = _G.extractPresentationPreferences = (emailText, journalMatches) ->
  _G.traceStep '🎯', 'Extracting presentation preferences', () ->
    void emailText
    extractPresentationCandidateFromRecall(journalMatches)

# ---------------------------------------------------------------------------
# Git-backed journal storage
# ---------------------------------------------------------------------------

export ensureJournalGitRepo = _G.ensureJournalGitRepoLib = (spawn, dbDir) ->
  { stat } = await import('fs/promises')
  try
    await stat(resolve(dbDir, '.git'))
    return # already has its own git repo
  catch
    ### not initialised yet ###

  await spawn('git', ['-C', dbDir, 'init'])
  await spawn('git', ['-C', dbDir, 'add', '-A'])
  commit = await spawn('git', ['-C', dbDir, 'commit', '-m', 'journal: initial commit', '--allow-empty'])
  _G.log 'git.init', { dbDir, code: commit.code }, 'memo'

export gitJournalCommit = _G.gitJournalCommitLib = (spawn, dbDir, message) ->
  await spawn('git', ['-C', dbDir, 'add', '-A'])
  commit = await spawn('git', ['-C', dbDir, 'commit', '-m', message, '--allow-empty'])
  _G.log 'git.commit', { dbDir, message, code: commit.code }, 'memo'
  commit

# ---------------------------------------------------------------------------
# Reinforcement — increment confirmed_count on a cited journal entry
# ---------------------------------------------------------------------------

export reinforceJournalEntry = _G.reinforceJournalEntryLib = (spawn, dbDir, memoDbPath, journalId) ->
  id = Number(journalId)
  if not Number.isFinite(id) or id < 0 then return

  _G.traceStep '🏆', 'Reinforcing journal entry', () ->
    # Read the raw YAML to find the entry and its current confirmed_count
    raw = ''
    try
      { readFile: rf } = await import('fs/promises')
      raw = await rf("#{memoDbPath}.yaml", 'utf8')
    catch
      return

    docs = raw.split(/(?:^|\n)---(?:\n|$)/).map((s) -> s.trim()).filter(Boolean)
    targetBody = null
    currentCount = 0
    existingMeta = {}

    for doc in docs
      try
        { YAML: Y } = await import('bun')
        parsed = Y.parse(doc) or {}
        if Number(parsed.id) === id
          targetBody = String(parsed.body or '')
          existingMeta = if (parsed.metadata and typeof parsed.metadata === 'object') then { ...parsed.metadata } else {}
          currentCount = Number(existingMeta.confirmed_count or 0)
          break
      catch
        ### skip ###

    if targetBody === null then return

    now = new Date().toISOString()
    tmpFile = resolve(dbDir, "memo-reinforce-#{id}-#{Date.now()}.yaml")
    mergedMeta = { ...existingMeta, ts: now, confirmed_count: currentCount + 1, last_confirmed_ts: now }
    yamlDoc = YAML.stringify({ id, metadata: mergedMeta, body: targetBody })

    await mkdir(dbDir, { recursive: true })
    { writeFile: wf, rm: rmf } = await import('fs/promises')
    await wf(tmpFile, yamlDoc, 'utf8')
    save = await spawn('memo', ['save', '-f', memoDbPath, tmpFile])
    await rmf(tmpFile, { force: true })

    if save.code !== 0
      console.error 'Failed to reinforce journal entry.'
      if save.stderr then console.error save.stderr.trim()
      return

    # Reindex so the updated metadata is reflected in vector search
    await reindexMemoDb(spawn, memoDbPath)
    _G.log 'journal.reinforced', { id, confirmedCount: currentCount + 1 }, 'memo'
