import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.recommendActionMicroagent = async (emailText, journalMatches) => {
  return _G.traceStep('🗺️', 'Generating recommendation', async () => {
    const microagent = await Agent.factory({
      system_prompt: `
You recommend which available-operations to apply to incoming-email-content, based on past actions logged to journal-context.
You should reason whether journal-context is relevant to the incoming-email-content; although the journal-context is the closest semantic match for incoming-email-content, it may be a poor/irrelevant match--ignore the journal-context in that case.`,
      output_tool: {
        description: 'Give your recommendation',
        parameters: {
          journal_id: { type: 'integer', description: `The ID found in the journal-context that you used. This must be a valid citation; do not make up a value. If journal-context was empty or irrelevant to the email, return 0.` },
          operations: {
            type: 'string', description: `Which operation(s) do you recommend for the user to take?
If uncertain, safest option combo is: mark as read + move to archive.
A move operation must match existing destination (case-sensitive).
**NOTICE:** You may choose more than one operation. Often a folder move is preceded by mark-as-read, but it does not have to be.
If you choose delete, it is mutually-exclusive with any other option.` },
          rationale: { type: 'string', description: 'Concisely list the factors in your decision (≤25 words)' },
        },
        required: ['journal_id', 'operations', 'rationale'],
      },
    });

    const prompt = `
<incoming-email-content>
${_G.xmlEscape(emailText)}
</incoming-email-content>

<journal-context>
${_G.xmlEscape(journalMatches || 'No relevant journal entry found.')}
</journal-context>

<available-operations>
mark as read/unread, delete, move to {{destination}}.
</available-operations>

<valid-move-destinations>
${_G.xmlEscape(_G.renderMoveFolderChoicesLib(_G.cachedMoveFolders))}
</valid-move-destinations>
`;

    const result = await microagent.run({ prompt });
    const ref = result.journal_id == 0 ? 'Guess' : `Journal ${result.journal_id}`;
    const output = { ref, ...result };
    _G.log('microagent.result', {
      name: 'recommendActionMicroagent',
      input: { emailText, journalMatches },
      output,
      rawOutput: result,
    }, 'microagent');
    return output;
  });
}


