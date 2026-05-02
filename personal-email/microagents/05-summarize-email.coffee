import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.mjs'

_G.summarizeEmailMicroagent = (emailText, presentationPreferences = '') ->
  _G.traceStep '🧠', 'Summarizing email', ->
    microagent = await Agent.factory
      system_prompt: """
      You summarize emails for fast human triage.
      Be concise and factual.
      Sacrifice grammar for concision.
      """
      output_tool:
        parameters:
          headline:
            type: 'string'
            description: 'a compact one-line headline for email-content'
          description:
            type: 'string'
            description: 'main summary for email-content, after having any formatting-instructions applied'
        required: [
          'headline'
          'description'
        ]

    prompt = """
      <formatting-instructions>
      #{_G.xmlEscape String(presentationPreferences or '')}
      </formatting-instructions>

      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>
      """

    result = await microagent.run { prompt }
    output =
      headline: String(result.headline or '').trim()
      description: String(result.description or '').trim()
      text: "Summary: #{result.headline}\n\n#{result.description}"
    _G.log 'microagent.result',
      name: 'summarizeEmailMicroagent'
      input: { emailText, presentationPreferences }
      output: output.text
      rawOutput: result
    , 'microagent'
    output
