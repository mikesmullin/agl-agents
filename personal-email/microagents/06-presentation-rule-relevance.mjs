import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

_G.isRelevant = async (emailContent, ruleLogic) => {
  return _G.traceStep('✅', 'Checking preference relevance', async () => {
    const microagent = await Agent.factory({
      system_prompt: `You determine whether rule-logic is satisfied by the email-content.`,
      output_tool: {
        type: 'boolean',
        description: 'rule-logic satisfied?',
      },
    });

    const prompt = `
<rule-logic>
${_G.xmlEscape(ruleLogic)}
</rule-logic>

<email-content>
${_G.xmlEscape(emailContent)}
</email-content>
`;

    const result = await microagent.run({ prompt });
    _G.log('microagent.result', {
      name: 'isRelevant',
      input: { emailContent, ruleLogic },
      output: result,
    }, 'microagent');
    return result;
  });
}


