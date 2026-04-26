import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.summarizeEmailMicroagent = async (emailText, presentationPreferences = '') => {
  return _G.traceStep('🧠', 'Summarizing email', async () => {
    const microagent = await Agent.factory({
      system_prompt: `You summarize emails for fast human triage. Be concise and factual. Sacrifice grammar for concision.`,
      output_tool: {
        parameters: {
          headline: { type: 'string', description: 'a compact one-line headline for email-content' },
          description: { type: 'string', description: 'main summary for email-content, after having any formatting-instructions applied' },
        },
        required: ['headline', 'description'],
      },
    });

    const prompt = `
<formatting-instructions>
${_G.xmlEscape(String(presentationPreferences || ''))}
</formatting-instructions>

<email-content>
${_G.xmlEscape(emailText)}
</email-content>
`;

    const result = await microagent.run({ prompt });
    const output = {
      headline: String(result.headline || '').trim(),
      description: String(result.description || '').trim(),
      text: `Summary: ${result.headline}\n\n${result.description}`,
    };
    _G.log('microagent.result', {
      name: 'summarizeEmailMicroagent',
      input: { emailText, presentationPreferences },
      output: output.text,
      rawOutput: result,
    }, 'microagent');
    return output;
  });
}


