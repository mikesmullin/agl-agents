import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { _G } from './globals.coffee'

# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

###
 Parse a multi-document YAML file (memo journal format) into an array of
 entry objects: { id, metadata, body, fields }.
###
readJournalEntries = (memoDbPath) ->
  raw = ''
  try
    raw = await readFile("#{memoDbPath}.yaml", 'utf8')
  catch
    return []

  docs = raw.split(/(?:^|\n)---(?:\n|$)/).map((s) -> s.trim()).filter(Boolean)
  entries = []

  for doc in docs
    try
      parsed = YAML.parse(doc) or {}
      if not parsed or typeof parsed isnt 'object' then continue
      id = Number(parsed.id ? -1)
      if not Number.isFinite(id) or id < 0 then continue
      body = String(parsed.body or '')
      entries.push({
        id,
        metadata: parsed.metadata or {},
        body,
        fields: parseBodyFields(body),
      })
    catch
      # skip malformed documents

  entries

# Parse the key: value lines in a journal entry body into a plain object.
parseBodyFields = (bodyText) ->
  fields = {}
  currentKey = ''
  for rawLine in String(bodyText or '').split('\n')
    line = rawLine.replace(/\r$/, '')
    kv = line.match(/^\s*([a-zA-Z_][a-zA-Z0-9_-]*):\s*(.*)$/)
    if kv
      currentKey = kv[1]
      fields[currentKey] = String(kv[2] or '').trim()
      continue
    if currentKey and /^\s+\S/.test(line)
      fields[currentKey] = "#{fields[currentKey]} #{line.trim()}".trim()
  fields

# Format a raw entry as context text for the LLM, matching buildJournalContext format.
formatEntryAsContext = (entry) ->
  f = entry.fields
  JSON.stringify({
    id: entry.id,
    summary: f.summary or '',
    keywords: if Array.isArray(entry.metadata?.keywords) then entry.metadata.keywords.join(', ') else '',
    action_taken: f.action_taken or '',
    factors: f.factors or '',
    rule: f.rule or '',
    applies_if: f.applies_if or '',
    match_criteria: f.match_criteria or '',
    sender_email: String(entry.metadata?.sender_email or ''),
    raw_excerpt: entry.body.slice(0, 1200),
  })

# ---------------------------------------------------------------------------
# Strategy 1 — Exact sender match (deterministic)
# ---------------------------------------------------------------------------

recallBySender = (spawn, memoDbPath, incomingSenderEmail) ->
  senderEmail = String(incomingSenderEmail or '').toLowerCase().trim()
  if not senderEmail then return []

  filter = "sender_email: #{JSON.stringify(senderEmail)}"
  result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--filter', filter, '--yaml', senderEmail])
  if result.code isnt 0 then return []
  _G.parseMemoRecallResults(result.stdout)

# ---------------------------------------------------------------------------
# Strategy 2 — Keyword full-text search (hybrid)
# ---------------------------------------------------------------------------

recallByKeywords = (spawn, memoDbPath, fingerprint, emailText) ->
  rawKw = String(fingerprint?.keywords or '')
    .split(',')
    .map((k) -> k.trim().toLowerCase())
    .filter(Boolean)

  if not rawKw.length then return []

  uniqueKeywords = [...new Set(rawKw)].slice(0, 8)
  results = await Promise.all(uniqueKeywords.map((keyword) ->
    filter = "keywords: {$contains: #{JSON.stringify(keyword)}}"
    r = await spawn('memo', [
      'recall', '-f', memoDbPath, '-k', '5', '--filter', filter, '--yaml', emailText.slice(0, 1500),
    ])
    if r.code is 0 then _G.parseMemoRecallResults(r.stdout) else []
  ))

  byId = new Map()
  for rows in results
    for row in rows
      if row.id <= 0 then continue
      existing = byId.get(row.id)
      if not existing or row.score > existing.score
        byId.set(row.id, row)

  [...byId.values()].sort((a, b) -> b.score - a.score).slice(0, 5)

# ---------------------------------------------------------------------------
# Strategy 3 — Q&A semantic search via memo recall
# ---------------------------------------------------------------------------

recallByQA = (spawn, memoDbPath, fingerprint) ->
  query = [
    fingerprint?.sender_offers,
    fingerprint?.sender_expects,
    fingerprint?.reader_value,
  ].filter((s) -> s and s isnt 'none').join('. ')

  if not query.trim() then return []

  result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--yaml', query])
  if result.code isnt 0 then return []
  _G.parseMemoRecallResults(result.stdout)

# ---------------------------------------------------------------------------
# Strategy 4 — Whole-email vector search (existing)
# ---------------------------------------------------------------------------

recallByVector = (spawn, memoDbPath, emailText) ->
  query = emailText.slice(0, 1500)
  result = await spawn('memo', ['recall', '-f', memoDbPath, '-k', '5', '--yaml', query])
  if result.code isnt 0 then return []
  _G.parseMemoRecallResults(result.stdout)

# ---------------------------------------------------------------------------
# Result fusion
# ---------------------------------------------------------------------------

fuseResults = (allEntries, s1Entries, s2Entries, s3Results, s4Results) ->
  # Build a lookup map by ID from raw journal
  byId = new Map(allEntries.map((e) -> [e.id, e]))

  # Collect candidate IDs and count how many strategies returned each
  strategyCounts = new Map()
  add = (id) -> strategyCounts.set(id, (strategyCounts.get(id) or 0) + 1)

  for r in s1Entries then if r.id > 0 then add(r.id)
  for r in s2Entries then if r.id > 0 then add(r.id)
  for r in s3Results then if r.id > 0 then add(r.id)
  for r in s4Results then if r.id > 0 then add(r.id)

  # Build unified sorted list
  candidates = [...strategyCounts.entries()]
    .map(([id, strategies]) ->
      entry = byId.get(id)
      {
        id,
        strategies,
        confirmedCount: Number(entry?.metadata?.confirmed_count or 0),
        entry,
      }
    )
    .filter((c) -> c.entry) # skip IDs not found in raw journal (stale recall results)
    .sort((a, b) ->
      b.strategies - a.strategies or
      b.confirmedCount - a.confirmedCount or
      b.id - a.id # higher id = more recent
    )
    .slice(0, 10)

  candidates.map((c) -> c.entry)

# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

###
 Run all four retrieval strategies in parallel and return a fused, ranked
 list of up to 10 journal entries as a context string for the LLM.
###
export hybridJournalRecall = _G.hybridJournalRecallLib = (spawn, memoDbPath, emailText, fingerprint, envelope) ->
  _G.traceStep('🔎', 'Hybrid journal recall', () ->
    allEntries = await readJournalEntries(memoDbPath)
    if not allEntries.length
      return { context: 'No relevant journal entry found.', entries: [] }

    s1 = await recallBySender(spawn, memoDbPath, envelope?.senderEmail)
    s2 = await recallByKeywords(spawn, memoDbPath, fingerprint, emailText)
    [s3, s4] = await Promise.all([
      recallByQA(spawn, memoDbPath, fingerprint),
      recallByVector(spawn, memoDbPath, emailText),
    ])

    _G.log('recall.strategies', {
      s1: s1.map((r) -> r.id),
      s2: s2.map((r) -> r.id),
      s3: s3.map((r) -> r.id),
      s4: s4.map((r) -> r.id),
    }, 'memo')

    fused = fuseResults(allEntries, s1, s2, s3, s4)

    context = if fused.length
      "[#{fused.map(formatEntryAsContext).join(',\n')}]"
    else
      'No relevant journal entry found.'

    { context, entries: fused }
  )

###
 Read every journal entry from the raw YAML file and return them as an array.
 Exported so agent.mjs can use it for the re-index relevance filter loop.
###
export readAllJournalEntries = _G.readAllJournalEntriesLib = (memoDbPath) ->
  readJournalEntries(memoDbPath)

# Format a single raw journal entry as a text block for LLM input.
export formatJournalEntryForPrompt = _G.formatJournalEntryForPromptLib = (entry) ->
  "[Journal #{entry.id}]\n#{entry.body.trim()}"
