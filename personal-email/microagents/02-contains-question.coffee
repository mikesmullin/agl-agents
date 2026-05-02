import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.containsQuestionMicroagent = (instruction) ->
  _G.traceStep '❓', 'Checking if instruction is a question', ->
    microagent = await Agent.factory
      system_prompt: 'Determine whether the user-instruction contains a question.'
      output_tool:
        type: 'boolean'
        description: 'Is asking a question?'

    prompt = """
      <user-instruction>
      #{_G.xmlEscape instruction}
      </user-instruction>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'containsQuestionMicroagent'
      input: { instruction }
      output: result
    , 'microagent'
    result
