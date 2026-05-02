import Agent from '../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../lib/globals.mjs'
import '../lib/color.mjs'
import '../lib/debug.mjs'
import '../lib/spawn.mjs'
import '../lib/text.mjs'
import '../lib/validate.mjs'
import '../lib/voice.mjs'
import './microagents/00-fingerprint-email.mjs'
import './microagents/01-recommend-action.mjs'
import './microagents/02-contains-question.mjs'
import './microagents/04-answer-question-from-email.mjs'
import './microagents/05-summarize-email.mjs'
import './microagents/06-presentation-rule-relevance.mjs'
import './microagents/07-execute-memo-instruction.mjs'
import './microagents/08-execute-instruction.mjs'
import './microagents/09-build-journal-entry.mjs'
import './microagents/10-build-presentation-entry.mjs'
import './microagents/14-journal-entry-relevance-filter.mjs'
import './microagents/15-consolidate-journal-group.mjs'
import '../lib/email-adapter.mjs'
import '../lib/html-email.mjs'
import '../lib/memo.mjs'
import '../lib/recall.mjs'
import { loadPerceptionConfig } from '../lib/perception.mjs'
import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { mkdir } from 'fs/promises'
import { createInterface } from 'readline/promises'
import { resolve } from 'path'

Agent.default.model = _G.MODEL
Agent.default.context_window = _G.CONTEXT_WINDOW

# Load journal config from config.yaml (with safe defaults)
journalConfig = { confidence_threshold: 60 }
try
  configText = await readFile(resolve(process.cwd(), 'config.yaml'), 'utf8')
  cfg = YAML.parse(configText or '') ? {}
  if cfg?.journal and typeof cfg.journal is 'object'
    journalConfig = { ...journalConfig, ...cfg.journal }
catch
  # use defaults

CONFIDENCE_THRESHOLD = Number(journalConfig.confidence_threshold ? 60)

await mkdir(_G.DB_DIR, { recursive: true })

# Ensure the journal db directory is git-tracked
await _G.ensureJournalGitRepoLib(_G.spawn, _G.DB_DIR)

# --perception flag: replace readline with voice-driven input
usePerception = process.argv.includes('--perception')
perception = null
if usePerception
  try
    perception = await loadPerceptionConfig(process.cwd())
    console.log('🎤 Perception mode active — listening for voice commands.')
  catch e
    console.error("⚠️  Failed to load personal-email/config.yaml perception config: #{e?.message or e}. Falling back to keyboard input.")

rl = createInterface({ input: process.stdin, output: process.stdout })

shuttingDown = false

beginGracefulShutdown = ->
  if shuttingDown
    process.exit(130)
  shuttingDown = true
  console.log('\nInterrupt received. Stopping active prompt and syncing pending mutations...')
  try
    rl.close()
    perception?.cancelActiveQuestion?()
    _G.stopSpeaking()
    await _G.endEmailTransaction()
  finally
    process.exit(130)

process.on('SIGINT', beginGracefulShutdown)

###*
 * Unified prompt: uses perception-voice when active, otherwise readline.
 * In perception mode, unmapped utterances are shown as hints and the prompt
 * re-displays until a mapped word is spoken.
###
ask = (promptText) ->
  if shuttingDown
    return ''
  if perception
    try
      return await perception.question(promptText, rl)
    catch error
      if (error?.code is 'ABORT_ERR' or error?.code is 'ERR_USE_AFTER_CLOSE') and shuttingDown
        return ''
      throw error
  try
    return await rl.question(promptText)
  catch error
    if error?.code is 'ERR_USE_AFTER_CLOSE' and shuttingDown
      return ''
    throw error

# Cache-busting reload of all microagent modules so edits to their
# prompts/logic take effect without restarting the agent process.
MICROAGENT_FILES = [
  '00-fingerprint-email.mjs'
  '01-recommend-action.mjs'
  '02-contains-question.mjs'
  '04-answer-question-from-email.mjs'
  '05-summarize-email.mjs'
  '06-presentation-rule-relevance.mjs'
  '07-execute-memo-instruction.mjs'
  '08-execute-instruction.mjs'
  '09-build-journal-entry.mjs'
  '10-build-presentation-entry.mjs'
  '14-journal-entry-relevance-filter.mjs'
  '15-consolidate-journal-group.mjs'
]
reloadMicroagents = ->
  t = Date.now()
  for f in MICROAGENT_FILES
    # Bare absolute path + ?t= is the correct cache-bust in Bun.
    # file:// URLs have their query string stripped during resolution,
    # so they do NOT produce a new cache entry.
    await import("#{import.meta.dir}/microagents/#{f}?t=#{t}")

cliArgs = process.argv.slice(2).filter((a) -> a isnt '--perception')
since = String(cliArgs[0] or '').trim() or undefined

pageIds = []
pageIndex = 0
shouldStop = false

await _G.loadMoveFolderCacheLib()

await _G.pullBatchLib(_G.spawn, { limit: _G.PULL_BATCH_SIZE, since, log: _G.log })

pageIds = await _G.loadPageIdsLib(_G.spawn, { limit: _G.PAGE_SIZE })

emailLoop: while true
  _G.log('loop.begin')

  if pageIndex >= pageIds.length
    _G.log('page.boundary', { pageSize: pageIds.length })
    await _G.pullBatchLib(_G.spawn,
      limit: _G.PULL_BATCH_SIZE
      since: since
      log: _G.log
      traceLabel: 'Refreshing emails at page boundary'
    )

    pageIds = await _G.loadPageIdsLib(_G.spawn,
      limit: _G.PAGE_SIZE
      traceLabel: 'Reloading unread inbox page'
    )
    pageIndex = 0

  emailId = pageIds[pageIndex] ? null
  _G.log('loop.firstEmail', { emailId })
  if not emailId
    console.log('No unread emails remain after refresh. Running graceful shutdown...')
    await _G.endEmailTransaction()
    break

  { envelope, summaryInput } = await _G.loadDecisionEmail(emailId)

  # Step 2: Fingerprint the email (keywords + intent questions) — used by recall strategies 2 & 3
  fingerprint = await _G.fingerprintEmailMicroagent(summaryInput)

  # Extract sender parts from envelope for strategy 1
  senderEnvelope = do ->
    raw = String(envelope.from or '')
    match = raw.match(/<([^>]+)>/) or raw.match(/(\S+@\S+)/)
    senderEmail = (match?.[1] or '').toLowerCase().trim()
    { senderEmail }

  # Step 3: Multi-strategy retrieval
  { context: journalContext } = await _G.hybridJournalRecallLib(
    _G.spawn, _G.MEMO_DB, summaryInput, fingerprint, senderEnvelope,
  )

  # Presentation recall (unchanged)
  presentationMatches = await _G.recallJournal(_G.spawn, _G.PRESENTATION_MEMO_DB, summaryInput)

  presentationCandidate = await _G.extractPresentationPreferences(summaryInput, presentationMatches)

  usePresentationPreferences = if presentationCandidate?.has_formatting_instructions
    await _G.isRelevant(summaryInput, presentationCandidate.applies_if)
  else
    false

  # Step 4: Summarize
  emailSummary = await _G.summarizeEmailMicroagent(
    summaryInput,
    _G.optionalText(usePresentationPreferences, presentationCandidate?.formatting_instructions),
  )

  # Step 5: Recommend (now includes confidence)
  recommendResult = await _G.recommendActionMicroagent(summaryInput, journalContext)
  confidence = recommendResult.confidence ? 0
  lowConfidence = confidence < CONFIDENCE_THRESHOLD
  confidenceLabel = if lowConfidence
    "#{_G.ANSI.dim}(confidence: #{confidence}% ⚠️ uncertain)#{_G.ANSI.reset}"
  else
    "#{_G.ANSI.dim}(confidence: #{confidence}%)#{_G.ANSI.reset}"
  recommendation = "(#{recommendResult.ref}) #{recommendResult.operations}. #{recommendResult.rationale}."

  ###await### _G.speakText("#{emailSummary.headline}. recommend: #{recommendResult.operations}") # speak (backgrounded, parallelized)

  presentationPreferencesText = _G.optionalText(
    usePresentationPreferences,
    presentationCandidate?.formatting_instructions,
  )

  # Step 6: Display
  console.log """

    ========== NEXT EMAIL ==========
    #{_G.ANSI.violet}#{_G.ANSI.bold}From:#{_G.ANSI.reset} #{envelope.from}
    #{_G.ANSI.violet}#{_G.ANSI.bold}Subj:#{_G.ANSI.reset} #{envelope.subject}
    #{_G.ANSI.violet}#{_G.ANSI.bold}Date:#{_G.ANSI.reset} #{envelope.date}
    🗣️ #{emailSummary.text}#{if presentationPreferencesText then "\n\nApplied preferences:\n#{presentationPreferencesText}" else ''}

    Recommended action:
    #{String(recommendation or '')} #{confidenceLabel}
    ===============================
    """

  # Step 7: Operator input loop
  instruction = ''
  instructionOrRecommendation = ''
  while true
    instruction = await ask(
      "#{_G.ANSI.cyan}#{_G.ANSI.bold}🤖 What would you like to do?#{_G.ANSI.reset}\n#{_G.ANSI.dim}(proceed, skip, refresh, reload, quit)> #{_G.ANSI.reset}",
    )
    _G.stopSpeaking()
    if shuttingDown
      shouldStop = true
      break
    if not instruction.trim()
      continue
    normalizedInstruction = instruction.trim().toLowerCase()
    if normalizedInstruction is 'p'
      instruction = 'proceed'
      normalizedInstruction = 'proceed'
    commandWords = new Set(normalizedInstruction.match(/[a-z]+/g) or [])

    if normalizedInstruction is 'quit' or commandWords.has('quit')
      console.log('Graceful exit requested. Running queued mutation sync...')
      await _G.endEmailTransaction()
      console.log('Stopping email handler loop.')
      shouldStop = true
      break

    if normalizedInstruction is 'skip'
      console.log('Skipping email.')
      pageIndex += 1
      continue emailLoop

    if normalizedInstruction is 'refresh'
      console.log('Refreshing email evaluation from disk...')
      continue emailLoop

    if normalizedInstruction is 'reload'
      reloadTrace = _G.traceStart('🔄', 'Reloading microagent modules from disk')
      await reloadMicroagents()
      reloadTrace.traceEnd()
      console.log('Microagents reloaded. Re-evaluating email...')
      continue emailLoop

    instructionOrRecommendation = instruction
    if normalizedInstruction is 'proceed'
      console.log('Proceed mode: applying recommended action.')
      instructionOrRecommendation = recommendation

      proceedTrace = _G.traceStart('⚙️', 'Executing recommended action')
      execution = await _G.executeInstructionMicroagent(emailId, String(recommendation or ''))
      proceedTrace.traceEnd()

      console.log("#{if execution.success then '✅' else '❌'} #{execution.summary}")
      _G.log('instruction.executed.proceed',
        success: execution.success
        emailId: emailId
        summary: execution.summary
      )

      # Reinforcement: increment confirmed_count on the cited journal entry
      if recommendResult.journal_id > 0
        await _G.reinforceJournalEntryLib(
          _G.spawn, _G.DB_DIR, _G.MEMO_DB, recommendResult.journal_id,
        )

      pageIndex += 1
      continue emailLoop

    if /\bpatterns?\b/i.test(instruction.trim())
      # --- MERGE MODE ---
      console.log('\n🔀 Merge mode. Describe which journal rules to select, or type "merge" to proceed to consolidation.\n')
      mergeRelevant = []
      criteriaHistory = []
      allMergeEntries = await _G.readAllJournalEntriesLib(_G.MEMO_DB)

      mergeSelectionLoop: while true
        criteria = await ask(
          "#{_G.ANSI.dim}Select criteria (or \"merge\" to proceed)> #{_G.ANSI.reset}",
        )
        cTrimmed = criteria.trim()
        if not cTrimmed then continue
        if cTrimmed.toLowerCase() is 'merge' then break
        criteriaHistory.push(cTrimmed)

        console.log("\n🔄 Scanning journal for entries matching: \"#{cTrimmed}\"\n")
        for entry in allMergeEntries
          if mergeRelevant.some((r) -> r.entry.id is entry.id) then continue
          entryText = _G.formatJournalEntryForPromptLib(entry)
          process.stdout.write("  Checking entry id=#{entry.id}…")
          filter = await _G.journalEntryRelevanceFilterMicroagent(
            summaryInput, '', '', entryText, cTrimmed,
          )

          if not filter.relevant
            console.log(" score=#{filter.confidence} — not relevant.")
            continue

          console.log(" score=#{filter.confidence}\n")
          console.log("#{_G.ANSI.dim}#{'─'.repeat(60)}#{_G.ANSI.reset}")
          console.log(entryText)
          console.log("#{_G.ANSI.dim}#{'─'.repeat(60)}#{_G.ANSI.reset}\n")

          verdict = await ask(
            "#{_G.ANSI.dim}Include this entry in merge? [Y/n/q]> #{_G.ANSI.reset}",
          )
          v = verdict.trim().toLowerCase()
          console.log('')
          if v is 'q'
            continue mergeSelectionLoop
          else if v isnt 'n'
            mergeRelevant.push({ entry, confidence: filter.confidence })

      if not mergeRelevant.length
        console.log('No entries selected for merge.\n')
      else
        patternSummary = criteriaHistory.join('; ')
        journalEntriesText = mergeRelevant
          .map(({ entry }) -> _G.formatJournalEntryForPromptLib(entry))
          .join('\n\n')

        extraInstructions = ''
        while true
          consolidation = await _G.consolidateJournalGroupMicroagent(
            summaryInput, patternSummary, patternSummary, journalEntriesText, extraInstructions,
          )
          supersedes = if Array.isArray(consolidation.entry_ids_to_supersede)
            consolidation.entry_ids_to_supersede
          else
            []
          console.log """

            📦 Proposed consolidation based on: "#{patternSummary}"

              Consolidated rule:   #{consolidation.consolidated_rule}
              Match criteria:      #{consolidation.consolidated_match_criteria}
              Action:              #{consolidation.consolidated_action}
              Rationale:           #{consolidation.consolidated_rationale}
              Supersedes entries:  [#{supersedes.map((id) -> "id=#{id}").join(', ')}]
            """
          approval = await ask(
            "#{_G.ANSI.dim}Apply this consolidation? (yes / no / edit)> #{_G.ANSI.reset}",
          )
          ans = approval.trim().toLowerCase()

          if ans is 'yes' or ans is 'y'
            mergedKeywords = [...new Set([
              ...mergeRelevant.flatMap(({ entry }) ->
                kws = entry.metadata?.keywords
                if Array.isArray(kws) then kws else String(kws or '').split(',').map((s) -> s.trim()).filter(Boolean)
              ),
            ])]
            mostRecentEntry = mergeRelevant.reduce(
              (best, { entry }) -> if entry.id > best.id then entry else best,
              mergeRelevant[0].entry,
            )
            await _G.saveJournalEntry(_G.spawn, _G.DB_DIR, _G.MEMO_DB,
              summary: consolidation.consolidated_rule
              keywords: mergedKeywords.join(', ')
              action_taken: consolidation.consolidated_action
              factors: consolidation.consolidated_rationale
              sender_email: String(mostRecentEntry.metadata?.sender_email or '')
              sender_offers: ''
              sender_expects: ''
              reader_value: ''
              match_criteria: consolidation.consolidated_match_criteria
              rule: consolidation.consolidated_rule
              applies_if: ''
            )
            if supersedes.length > 0
              await _G.spawn('memo', ['delete', '-f', _G.MEMO_DB, ...supersedes.map(String)])
            await _G.reindexMemoDbLib(_G.spawn, _G.MEMO_DB)
            console.log('✅ Journal merge complete.\n')
            break
          else if ans is 'no' or ans is 'n'
            console.log('Merge skipped.\n')
            break
          else if ans is 'edit' or ans.startsWith('edit ')
            extraInstructions = if ans.startsWith('edit ')
              ans.slice(5).trim()
            else
              await ask("#{_G.ANSI.dim}Additional instructions> #{_G.ANSI.reset}")
          else
            console.log('Unrecognized. Merge skipped.\n')
            break
      continue

    if /\b(memo|memos|journal|journals)\b/i.test(instruction)
      memoExec = await _G.executeMemoInstructionMicroagent(instruction)

      console.log("#{if memoExec.success then '✅' else '❌'} #{memoExec.summary}")
      _G.log('memo.instruction.executed', { success: memoExec.success, summary: memoExec.summary })
      continue

    hasQuestion = await _G.containsQuestionMicroagent(instruction)

    if hasQuestion
      answer = await _G.answerQuestionFromEmailMicroagent(summaryInput, instruction)
      _G.printRobotAnswer(answer)
      continue

    break

  if shouldStop
    break

  # Step 8: Execute instruction
  instructionTrace = _G.traceStart('⚙️', 'Executing your instruction')
  execution = await _G.executeInstructionMicroagent(emailId, instruction)
  instructionTrace.traceEnd()

  console.log("#{if execution.success then '✅' else '❌'} #{execution.summary}")
  _G.log('instruction.executed', { success: execution.success, emailId, summary: execution.summary })
  pageIndex += 1

  # Step 9: Build and save journal entry
  journalEntry = await _G.buildJournalEntryMicroagent(
    summaryInput, instructionOrRecommendation, execution.summary,
  )

  await _G.saveJournalEntry(_G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry)

  # Presentation entry (unchanged)
  presentationEntry = await _G.hasFormattingInstructions(instruction, summaryInput)

  if presentationEntry?.has_formatting_instructions
    await _G.savePresentationEntry(_G.spawn, _G.DB_DIR, _G.PRESENTATION_MEMO_DB, presentationEntry)
  _G.log('journal.saved', { emailId, hasEntry: Boolean(journalEntry) })

rl.close()
