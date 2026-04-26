import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.answerQuestionFromEmailMicroagent = async (emailText, question) => {
  return _G.traceStep('🤖', 'Analyzing email for your question', async () => {
    const microagent = await Agent.factory({
      system_prompt: `Concisely answer user-question using email-content. If you are unable to, explain why.`,
      output_tool: {
        type: 'string',
        description: 'Answer to user-question',
      },
    });

    const prompt = `
<user-question>
${_G.xmlEscape(question)}
</user-question>

<email-content>
${_G.xmlEscape(emailText)}
</email-content>
`;

    const result = await microagent.run({ prompt });
    _G.log('microagent.result', {
      name: 'answerQuestionFromEmailMicroagent',
      input: { emailText, question },
      output: result,
    }, 'microagent');
    return result;
  });
}
