import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.fingerprintEmailMicroagent = async (emailText) => {
  return _G.traceStep('🔬', 'Fingerprinting email', async () => {
    const microagent = await Agent.factory({
      system_prompt: `You extract keywords and characterize the intent of email-content for use in memory retrieval.`,
      output_tool: {
        parameters: {
          keywords: {
            type: 'string',
            description: 'One comma-separated keyword list combining high-signal identifiers and useful topic labels for deterministic matching. Include brand names, proper nouns, domain fragments, model numbers, subject terms, and topic labels.',
          },
          sender_offers: {
            type: 'string',
            description: 'One sentence: what the sender provides or sells (≤15 words). Use "none" if not applicable.',
          },
          sender_expects: {
            type: 'string',
            description: 'One sentence: the call to action the sender wants the reader to take (≤15 words).',
          },
          reader_value: {
            type: 'string',
            description: 'One sentence: the potential benefit or value to the reader, or "none" (≤15 words).',
          },
        },
        required: ['keywords', 'sender_offers', 'sender_expects', 'reader_value'],
      },
    });

    const prompt = `
<email-content>
${_G.xmlEscape(emailText)}
</email-content>
`;

    const result = await microagent.run({ prompt });
    _G.log('microagent.result', {
      name: 'fingerprintEmailMicroagent',
      input: { emailText },
      output: result,
    }, 'microagent');
    return result;
  });
};
