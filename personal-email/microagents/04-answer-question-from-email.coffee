import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.answerQuestionFromEmailMicroagent = (emailText, question) ->
  _G.traceStep '🤖', 'Analyzing email for your question', ->
    microagent = await Agent.factory
      system_prompt: """
        Concisely answer user-question using email-content. If you are unable to, explain why.
        Do not include URLs or hyperlinks in your answer text — your response is read aloud via text-to-speech and spelling out URLs sounds terrible. Instead, acknowledge that a link exists (e.g. "there is a link to the event page") and use the open_url tool only if the user explicitly asks to open or browse the link.
        """
      output_tool:
        type: 'string'
        description: 'Answer to user-question'

    microagent.Tool 'open_url', 'Open a URL in the default system browser. Only use this tool when the user explicitly asks to open, view, or browse a link.',
      url:
        type: 'string'
        description: 'The fully-qualified URL to open.'
    , ['url'], (_ctx, { url }) ->
        r = await _G.spawn 'xdg-open', [String(url or '')]
        if r.code is 0 then "Opened #{url}" else "Failed to open: #{r.stderr or r.stdout}"

    prompt = """
      <user-question>
      #{_G.xmlEscape question}
      </user-question>

      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'answerQuestionFromEmailMicroagent'
      input: { emailText, question }
      output: result
    , 'microagent'
    result
