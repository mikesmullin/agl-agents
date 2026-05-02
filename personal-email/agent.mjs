import Agent from '../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../lib/globals.mjs';
import '../lib/color.mjs';
import '../lib/debug.mjs';
import '../lib/spawn.mjs';
import '../lib/text.mjs';
import '../lib/validate.mjs';
import '../lib/voice.mjs';
import './microagents/00-fingerprint-email.mjs';
import './microagents/01-recommend-action.mjs';
import './microagents/02-contains-question.mjs';
import './microagents/04-answer-question-from-email.mjs';
import './microagents/05-summarize-email.mjs';
import './microagents/06-presentation-rule-relevance.mjs';
import './microagents/07-execute-memo-instruction.mjs';
import './microagents/08-execute-instruction.mjs';
import './microagents/09-build-journal-entry.mjs';
import './microagents/10-build-presentation-entry.mjs';
import './microagents/14-journal-entry-relevance-filter.mjs';
import './microagents/15-consolidate-journal-group.mjs';
import '../lib/email-adapter.mjs';
import '../lib/html-email.mjs';
import '../lib/memo.mjs';
import '../lib/recall.mjs';
import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { mkdir } from 'fs/promises';
import { createInterface } from 'readline/promises';
import { resolve } from 'path';

Agent.default.model = _G.MODEL;
Agent.default.context_window = _G.CONTEXT_WINDOW;

// Load journal config from config.yaml (with safe defaults)
let journalConfig = { confidence_threshold: 60 };
try {
	const configText = await readFile(resolve(process.cwd(), 'config.yaml'), 'utf8');
	const cfg = YAML.parse(configText || '') || {};
	if (cfg?.journal && typeof cfg.journal === 'object') {
		journalConfig = { ...journalConfig, ...cfg.journal };
	}
}
catch { /* use defaults */ }

const CONFIDENCE_THRESHOLD = Number(journalConfig.confidence_threshold ?? 60);

await mkdir(_G.DB_DIR, { recursive: true });

// Ensure the journal db directory is git-tracked
await _G.ensureJournalGitRepoLib(_G.spawn, _G.DB_DIR);

const rl = createInterface({ input: process.stdin, output: process.stdout });
const since = String(process.argv[2] || '').trim() || undefined;

let pageIds = [];
let pageIndex = 0;
let shouldStop = false;

await _G.loadMoveFolderCacheLib();

await _G.pullBatchLib(_G.spawn, { limit: _G.PULL_BATCH_SIZE, since, log: _G.log });

pageIds = await _G.loadPageIdsLib(_G.spawn, { limit: _G.PAGE_SIZE });

emailLoop:
while (true) {
	_G.log('loop.begin');

	if (pageIndex >= pageIds.length) {
		_G.log('page.boundary', { pageSize: pageIds.length });
		await _G.pullBatchLib(_G.spawn, {
			limit: _G.PULL_BATCH_SIZE,
			since,
			log: _G.log,
			traceLabel: 'Refreshing emails at page boundary',
		});

		pageIds = await _G.loadPageIdsLib(_G.spawn, {
			limit: _G.PAGE_SIZE,
			traceLabel: 'Reloading unread inbox page',
		});
		pageIndex = 0;
	}

	const emailId = pageIds[pageIndex] || null;
	_G.log('loop.firstEmail', { emailId });
	if (!emailId) {
		console.log('No unread emails remain after refresh. Running graceful shutdown...');
		await _G.endEmailTransaction();
		break;
	}

	const { envelope, summaryInput } = await _G.loadDecisionEmail(emailId);

	// Step 2: Fingerprint the email (keywords + intent questions) — used by recall strategies 2 & 3
	const fingerprint = await _G.fingerprintEmailMicroagent(summaryInput);

	// Extract sender parts from envelope for strategy 1
	const senderEnvelope = (() => {
		const raw = String(envelope.from || '');
		const match = raw.match(/<([^>]+)>/) || raw.match(/(\S+@\S+)/);
		const senderEmail = (match?.[1] || '').toLowerCase().trim();
		return { senderEmail };
	})();

	// Step 3: Multi-strategy retrieval
	const { context: journalContext } = await _G.hybridJournalRecallLib(
		_G.spawn, _G.MEMO_DB, summaryInput, fingerprint, senderEnvelope,
	);

	// Presentation recall (unchanged)
	const presentationMatches = await _G.recallJournal(_G.spawn, _G.PRESENTATION_MEMO_DB, summaryInput);

	const presentationCandidate = await _G.extractPresentationPreferences(summaryInput, presentationMatches);

	const usePresentationPreferences = presentationCandidate?.has_formatting_instructions
		? await _G.isRelevant(summaryInput, presentationCandidate.applies_if)
		: false;

	// Step 4: Summarize
	const emailSummary = await _G.summarizeEmailMicroagent(
		summaryInput,
		_G.optionalText(usePresentationPreferences, presentationCandidate?.formatting_instructions),
	);

	// Step 5: Recommend (now includes confidence)
	const recommendResult = await _G.recommendActionMicroagent(summaryInput, journalContext);
	const confidence = recommendResult.confidence ?? 0;
	const lowConfidence = confidence < CONFIDENCE_THRESHOLD;
	const confidenceLabel = lowConfidence
		? `${_G.ANSI.dim}(confidence: ${confidence}% ⚠️ uncertain)${_G.ANSI.reset}`
		: `${_G.ANSI.dim}(confidence: ${confidence}%)${_G.ANSI.reset}`;
	const recommendation = `(${recommendResult.ref}) ${recommendResult.operations}. ${recommendResult.rationale}.`;

	/*await*/ _G.speakText(`${emailSummary.headline}. recommend: ${recommendResult.operations}`); // speak (backgrounded, parallelized)

	const presentationPreferencesText = _G.optionalText(
		usePresentationPreferences,
		presentationCandidate?.formatting_instructions,
	);

	// Step 6: Display
	console.log(`
========== NEXT EMAIL ==========
${_G.ANSI.violet}${_G.ANSI.bold}From:${_G.ANSI.reset} ${envelope.from}
${_G.ANSI.violet}${_G.ANSI.bold}Subj:${_G.ANSI.reset} ${envelope.subject}
${_G.ANSI.violet}${_G.ANSI.bold}Date:${_G.ANSI.reset} ${envelope.date}
🗣️ ${emailSummary.text}${presentationPreferencesText ? `

Applied preferences:
${presentationPreferencesText}` : ''}

Recommended action:
${String(recommendation || '')} ${confidenceLabel}
===============================
`);

	// Step 7: Operator input loop
	let instruction = '';
	let instructionOrRecommendation = '';
	while (true) {
		instruction = await rl.question(
			`${_G.ANSI.cyan}${_G.ANSI.bold}🤖 What would you like to do?${_G.ANSI.reset}\n${_G.ANSI.dim}(proceed, skip, quit)> ${_G.ANSI.reset}`,
		);
		_G.stopSpeaking();
		if (!instruction.trim()) {
			continue;
		}
		if (instruction.trim().toLowerCase() === 'p') {
			instruction = 'proceed';
		}

		if (instruction.toLowerCase().includes('quit')) {
			console.log('Graceful exit requested. Running queued mutation sync...');
			await _G.endEmailTransaction();
			console.log('Stopping email handler loop.');
			shouldStop = true;
			break;
		}

		if (instruction.trim().toLowerCase() === 'skip') {
			console.log('Skipping email.');
			pageIndex += 1;
			continue emailLoop;
		}

		instructionOrRecommendation = instruction;
		if (instruction.toLowerCase().includes('proceed')) {
			console.log('Proceed mode: applying recommended action.');
			instructionOrRecommendation = recommendation;

			const proceedTrace = _G.traceStart('⚙️', 'Executing recommended action');
			const execution = await _G.executeInstructionMicroagent(emailId, String(recommendation || ''));
			proceedTrace.traceEnd();

			console.log(`${execution.success ? '✅' : '❌'} ${execution.summary}`);
			_G.log('instruction.executed.proceed', {
				success: execution.success,
				emailId,
				summary: execution.summary,
			});

			// Reinforcement: increment confirmed_count on the cited journal entry
			if (recommendResult.journal_id > 0) {
				await _G.reinforceJournalEntryLib(
					_G.spawn, _G.DB_DIR, _G.MEMO_DB, recommendResult.journal_id,
				);
			}

			pageIndex += 1;
			continue emailLoop;
		}

		if (/\bpatterns?\b/i.test(instruction.trim())) {
			// --- MERGE MODE ---
			console.log('\n🔀 Merge mode. Describe which journal rules to select, or type "merge" to proceed to consolidation.\n');
			const mergeRelevant = [];
			const criteriaHistory = [];
			const allMergeEntries = await _G.readAllJournalEntriesLib(_G.MEMO_DB);

			mergeSelectionLoop: while (true) {
				const criteria = await rl.question(
					`${_G.ANSI.dim}Select criteria (or "merge" to proceed)> ${_G.ANSI.reset}`,
				);
				const cTrimmed = criteria.trim();
				if (!cTrimmed) continue;
				if (cTrimmed.toLowerCase() === 'merge') break;
				criteriaHistory.push(cTrimmed);

				console.log(`\n🔄 Scanning journal for entries matching: "${cTrimmed}"\n`);
				for (const entry of allMergeEntries) {
					if (mergeRelevant.some((r) => r.entry.id === entry.id)) continue;
					const entryText = _G.formatJournalEntryForPromptLib(entry);
					process.stdout.write(`  Checking entry id=${entry.id}…`);
					const filter = await _G.journalEntryRelevanceFilterMicroagent(
						summaryInput, '', '', entryText, cTrimmed,
					);

					if (!filter.relevant) {
						console.log(` score=${filter.confidence} — not relevant.`);
						continue;
					}

					console.log(` score=${filter.confidence}\n`);
					console.log(`${_G.ANSI.dim}${'─'.repeat(60)}${_G.ANSI.reset}`);
					console.log(entryText);
					console.log(`${_G.ANSI.dim}${'─'.repeat(60)}${_G.ANSI.reset}\n`);

					const verdict = await rl.question(
						`${_G.ANSI.dim}Include this entry in merge? [Y/n/q]> ${_G.ANSI.reset}`,
					);
					const v = verdict.trim().toLowerCase();
					console.log('');
					if (v === 'q') {
						continue mergeSelectionLoop;
					}
					else if (v !== 'n') {
						mergeRelevant.push({ entry, confidence: filter.confidence });
					}
				}
			}

			if (!mergeRelevant.length) {
				console.log('No entries selected for merge.\n');
			}
			else {
				const patternSummary = criteriaHistory.join('; ');
				const journalEntriesText = mergeRelevant
					.map(({ entry }) => _G.formatJournalEntryForPromptLib(entry))
					.join('\n\n');

				let extraInstructions = '';
				while (true) {
					const consolidation = await _G.consolidateJournalGroupMicroagent(
						summaryInput, patternSummary, patternSummary, journalEntriesText, extraInstructions,
					);
					const supersedes = Array.isArray(consolidation.entry_ids_to_supersede)
						? consolidation.entry_ids_to_supersede
						: [];
					console.log(`
📦 Proposed consolidation based on: "${patternSummary}"

  Consolidated rule:   ${consolidation.consolidated_rule}
  Match criteria:      ${consolidation.consolidated_match_criteria}
  Action:              ${consolidation.consolidated_action}
  Rationale:           ${consolidation.consolidated_rationale}
  Supersedes entries:  [${supersedes.map((id) => `id=${id}`).join(', ')}]
`);
					const approval = await rl.question(
						`${_G.ANSI.dim}Apply this consolidation? (yes / no / edit)> ${_G.ANSI.reset}`,
					);
					const ans = approval.trim().toLowerCase();

					if (ans === 'yes' || ans === 'y') {
						const mergedKeywords = [...new Set([
							...mergeRelevant.flatMap(({ entry }) => {
								const kws = entry.metadata?.keywords;
								return Array.isArray(kws) ? kws : String(kws || '').split(',').map((s) => s.trim()).filter(Boolean);
							}),
						])];
						const mostRecentEntry = mergeRelevant.reduce(
							(best, { entry }) => entry.id > best.id ? entry : best,
							mergeRelevant[0].entry,
						);
						await _G.saveJournalEntry(_G.spawn, _G.DB_DIR, _G.MEMO_DB, {
							summary: consolidation.consolidated_rule,
							keywords: mergedKeywords.join(', '),
							action_taken: consolidation.consolidated_action,
							factors: consolidation.consolidated_rationale,
							sender_email: String(mostRecentEntry.metadata?.sender_email || ''),
							sender_offers: '',
							sender_expects: '',
							reader_value: '',
							match_criteria: consolidation.consolidated_match_criteria,
							rule: consolidation.consolidated_rule,
							applies_if: '',
						});
						if (supersedes.length > 0) {
							await _G.spawn('memo', ['delete', '-f', _G.MEMO_DB, ...supersedes.map(String)]);
						}
						await _G.reindexMemoDbLib(_G.spawn, _G.MEMO_DB);
						console.log('✅ Journal merge complete.\n');
						break;
					}
					else if (ans === 'no' || ans === 'n') {
						console.log('Merge skipped.\n');
						break;
					}
					else if (ans === 'edit' || ans.startsWith('edit ')) {
						extraInstructions = ans.startsWith('edit ')
							? ans.slice(5).trim()
							: await rl.question(`${_G.ANSI.dim}Additional instructions> ${_G.ANSI.reset}`);
					}
					else {
						console.log('Unrecognized. Merge skipped.\n');
						break;
					}
				}
			}
			continue;
		}

		if (/\b(memo|memos|journal|journals)\b/i.test(instruction)) {
			const memoExec = await _G.executeMemoInstructionMicroagent(instruction);

			console.log(`${memoExec.success ? '✅' : '❌'} ${memoExec.summary}`);
			_G.log('memo.instruction.executed', { success: memoExec.success, summary: memoExec.summary });
			continue;
		}

		const hasQuestion = await _G.containsQuestionMicroagent(instruction);

		if (hasQuestion) {
			const answer = await _G.answerQuestionFromEmailMicroagent(summaryInput, instruction);
			_G.printRobotAnswer(answer);
			continue;
		}

		break;
	}

	if (shouldStop) {
		break;
	}

	// Step 8: Execute instruction
	const instructionTrace = _G.traceStart('⚙️', 'Executing your instruction');
	const execution = await _G.executeInstructionMicroagent(emailId, instruction);
	instructionTrace.traceEnd();

	console.log(`${execution.success ? '✅' : '❌'} ${execution.summary}`);
	_G.log('instruction.executed', { success: execution.success, emailId, summary: execution.summary });
	pageIndex += 1;

	// Step 9: Build and save journal entry
	const journalEntry = await _G.buildJournalEntryMicroagent(
		summaryInput, instructionOrRecommendation, execution.summary,
	);

	await _G.saveJournalEntry(_G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry);

	// Presentation entry (unchanged)
	const presentationEntry = await _G.hasFormattingInstructions(instruction, summaryInput);

	if (presentationEntry?.has_formatting_instructions) {
		await _G.savePresentationEntry(_G.spawn, _G.DB_DIR, _G.PRESENTATION_MEMO_DB, presentationEntry);
	}
	_G.log('journal.saved', { emailId, hasEntry: Boolean(journalEntry) });
}

rl.close();