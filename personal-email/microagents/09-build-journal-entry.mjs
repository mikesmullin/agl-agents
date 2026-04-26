import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.buildJournalEntryMicroagent = async (emailText, instruction, executionOutcome) => {
  return _G.traceStep('📝', 'Building journal entry', async () => {
    const microagent = await Agent.factory({
      system_prompt: `Create a compact journal record describing how we handled one email-content.`,
      output_tool: {
        parameters: {
          summary: { type: 'string', description: 'Short description of the email content.' },
          keywords: { type: 'string', description: 'Comma-separated high-signal terms/entities for future matching.' },
          action_taken: { type: 'string', description: 'What action was taken.' },
          factors: {
            type: 'string', description: `Decision factors as concise phrases. ` +
              `Factors must preserve user rationale and decision criteria from instruction. ` +
              `If instruction includes numeric ranges/thresholds, include them explicitly in factors. ` +
              `If instruction includes conditional follow-up behavior, include that in factors. ` +
              `Keywords must be concise comma-separated terms/entities useful for future matching. ` +
              `Keep factors concise and semicolon-separated`
          },
        },
        required: ['summary', 'keywords', 'action_taken', 'factors'],
      },
    });

    const prompt = `
<email-content>
${_G.xmlEscape(emailText)}
</email-content>

<user-instruction>
${_G.xmlEscape(instruction)}
</user-instruction>

<execution-outcome>
${_G.xmlEscape(String(executionOutcome))}
</execution-outcome>
`;

    const result = await microagent.run({ prompt });
    _G.log('microagent.result', {
      name: 'buildJournalEntryMicroagent',
      input: { emailText, instruction, executionOutcome },
      output: result,
    }, 'microagent');
    return result;
  });
}
