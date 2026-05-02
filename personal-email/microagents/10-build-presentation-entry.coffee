import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.hasFormattingInstructions = (instruction, emailContent) ->
  _G.traceStep '🧩', 'Building presentation entry', ->
    microagent = await Agent.factory
      system_prompt: """
      From user-instructions, which may include a combination of (operational and formatting) 
      instructions, you will:
      - isolate the formatting instructions, if there are any.
      - isolate the constraints and conditions for when these formatting instructions should apply, if there are any.
      The email-content is provided only for resolving relative references from user-instructions 
      (for both formatting_instructions, and applies_if).
      """
      output_tool:
        parameters:
          has_formatting_instructions:
            type: 'boolean'
            description: 'Did the user-instructions contain formatting instructions?'
          applies_if:
            type: 'string'
            description: 'Isolated list of logical if-conditions which should be evaluated to determine when to apply these formatting instructions. If none, applies in all cases.'
          formatting_instructions:
            type: 'string'
            description: 'Isolated list of the formatting instructions provided by the user.'
        required: [
          'has_formatting_instructions'
          'applies_if'
          'formatting_instructions'
        ]

    prompt = """
      <user-instructions>
      #{_G.xmlEscape instruction}
      </user-instructions>

      <email-content>
      #{_G.xmlEscape emailContent}
      </email-content>
      """

    entry = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'hasFormattingInstructions'
      input: { instruction, emailContent }
      output: entry
    , 'microagent'
    entry
