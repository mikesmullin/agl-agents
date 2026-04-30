import Agent from '../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../lib/globals.mjs';
import '../lib/color.mjs';
import '../lib/debug.mjs';
import '../lib/spawn.mjs';
import '../lib/text.mjs';
import '../lib/validate.mjs';
import '../lib/voice.mjs';
import './microagents/01-recommend-action.mjs';
import './microagents/02-contains-question.mjs';
import './microagents/04-answer-question-from-email.mjs';
import './microagents/05-summarize-email.mjs';
import './microagents/06-presentation-rule-relevance.mjs';
import './microagents/07-execute-memo-instruction.mjs';
import './microagents/08-execute-instruction.mjs';
import './microagents/09-build-journal-entry.mjs';
import './microagents/10-build-presentation-entry.mjs';
import '../lib/email-adapter.mjs';
import '../lib/html-email.mjs';
import '../lib/memo.mjs';
import { mkdir } from 'fs/promises';
import { createInterface } from 'readline/promises';

Agent.default.model = _G.MODEL;
Agent.default.context_window = _G.CONTEXT_WINDOW;

await mkdir(_G.DB_DIR, { recursive: true });

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

	const journalMatches = await _G.recallJournal(_G.spawn, _G.MEMO_DB, summaryInput);
	_G.log('journal.recall', { matchedChars: journalMatches.length });
	const journalContext = _G.buildJournalContext(journalMatches);

	const presentationMatches = await _G.recallJournal(_G.spawn, _G.PRESENTATION_MEMO_DB, summaryInput);

	const presentationCandidate = await _G.extractPresentationPreferences(summaryInput, presentationMatches);

	const usePresentationPreferences = presentationCandidate?.has_formatting_instructions
		? await _G.isRelevant(summaryInput, presentationCandidate.applies_if)
		: false;

	const emailSummary = await _G.summarizeEmailMicroagent(
		summaryInput,
		_G.optionalText(usePresentationPreferences, presentationCandidate?.formatting_instructions),
	);

	const recommendResult = await _G.recommendActionMicroagent(summaryInput, journalContext);
	const recommendation = `(${recommendResult.ref}) ${recommendResult.operations}. ${recommendResult.rationale}.`;

	/*await*/ _G.speakText(`${emailSummary.headline}. recommend: ${recommendResult.operations}`); // speak (backgrounded, parallelized)

	const presentationPreferencesText = _G.optionalText(
		usePresentationPreferences,
		presentationCandidate?.formatting_instructions,
	);
	console.log(`
========== NEXT EMAIL ==========
${_G.ANSI.violet}${_G.ANSI.bold}From:${_G.ANSI.reset} ${envelope.from}
${_G.ANSI.violet}${_G.ANSI.bold}Subj:${_G.ANSI.reset} ${envelope.subject}
${_G.ANSI.violet}${_G.ANSI.bold}Date:${_G.ANSI.reset} ${envelope.date}
🗣️ ${emailSummary.text}${presentationPreferencesText ? `

Applied preferences:
${presentationPreferencesText}` : ''}

Recommended action:
${String(recommendation || '')}
===============================
`);

	let instruction = '';
	let instructionOrRecommendation = '';
	while (true) {
		instruction = await rl.question(
			`${_G.ANSI.cyan}${_G.ANSI.bold}🤖 What would you like to do?${_G.ANSI.reset}\n${_G.ANSI.dim}(proceed, quit)> ${_G.ANSI.reset}`,
		);
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

		instructionOrRecommendation = instruction;
		if (instruction.toLowerCase().includes('proceed')) {
			console.log('Proceed mode: applying recommended action without journal update.');
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
			pageIndex += 1;
			continue emailLoop;
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

	const instructionTrace = _G.traceStart('⚙️', 'Executing your instruction');
	const execution = await _G.executeInstructionMicroagent(emailId, instruction);
	instructionTrace.traceEnd();

	console.log(`${execution.success ? '✅' : '❌'} ${execution.summary}`);
	_G.log('instruction.executed', { success: execution.success, emailId, summary: execution.summary });
	pageIndex += 1;

	const journalEntry = await _G.buildJournalEntryMicroagent(summaryInput, instructionOrRecommendation, execution.summary);

	await _G.saveJournalEntry(_G.spawn, _G.DB_DIR, _G.MEMO_DB, journalEntry);

	const presentationEntry = await _G.hasFormattingInstructions(instruction, summaryInput);

	if (presentationEntry?.has_formatting_instructions) {
		await _G.savePresentationEntry(_G.spawn, _G.DB_DIR, _G.PRESENTATION_MEMO_DB, presentationEntry);
	}
	_G.log('journal.saved', { emailId, hasEntry: Boolean(journalEntry) });
}

rl.close();