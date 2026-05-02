import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.mjs'

_G.detectPatternHintMicroagent = (emailText, instruction) ->
  _G.traceStep '🔁', 'Detecting pattern hint', ->
    microagent = await Agent.factory
      system_prompt: """
      You decide whether user-instruction implies the operator has recognized a recurring pattern
      — language that could form the basis of a reusable if-then rule applied to future emails.
      """
      output_tool:
        parameters:
          pattern_detected:
            type: 'boolean'
            description: 'True if the instruction contains language implying a recurring condition or pattern (e.g. "again", "another one", "same vendor", "always", "whenever", "if X then Y", "every time"). False otherwise.'
          pattern_summary:
            type: 'string'
            description: 'One sentence describing the recurring pattern the operator seems to have noticed. Empty string if pattern_detected is false.'
          applies_if:
            type: 'string'
            description: 'A concise constraint string describing when this rule should apply (e.g. "when email is from vendor X selling product Y"). Empty string if pattern_detected is false.'
        required: [
          'pattern_detected'
          'pattern_summary'
          'applies_if'
        ]

    prompt = """
      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>

      <user-instruction>
      #{_G.xmlEscape instruction}
      </user-instruction>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'detectPatternHintMicroagent'
      input: { emailText, instruction }
      output: result
    , 'microagent'
    result
