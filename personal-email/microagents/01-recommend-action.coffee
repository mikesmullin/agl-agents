import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.recommendActionMicroagent = (emailText, journalMatches) ->
  _G.traceStep '🗺️', 'Generating recommendation', ->
    microagent = await Agent.factory
      system_prompt: """
        You recommend which available-operations to apply to incoming-email-content, 
        based on past actions logged to journal-context.
        You should reason whether journal-context is relevant to the incoming-email-content; 
        although the journal-context is the closest semantic match for incoming-email-content, 
        occasionally it may seem irrelevant; if so, and if the sender/vendor name is not matching, 
        then you may ignore the journal entry.
        """
      output_tool:
        description: 'Give your recommendation'
        parameters:
          journal_id:
            type: 'integer'
            description: 'The ID found in the journal-context that you used. This must be a valid citation; do not make up a value. If journal-context was empty or irrelevant to the email, return 0.'
          operations:
            type: 'string'
            description: """
              Which operation(s) do you recommend for the user to take?
              **IMPORTANT:** If journal-context recommends to delete the email, you MUST recommend delete (without archive).
              A move operation must use ONLY the folder name — the text appearing BEFORE the ' — ' separator in the destination list. Never include the separator or any description text after it.
              A move operation must match existing destination exactly (case-sensitive).
              """
          rationale:
            type: 'string'
            description: 'Concisely list the factors in your decision (especially if you deviate from journal, explain why) (≤25 words)'
          confidence:
            type: 'integer'
            description: 'Self-assessed confidence score 0–100 that this recommendation matches what the operator would choose. High confidence (≥80) means the journal entry is a strong, clear match. Low confidence (<60) means the email is ambiguous or no journal entry applies.'
        required: ['journal_id', 'operations', 'rationale', 'confidence']

    prompt = """
      <incoming-email-content>
      #{_G.xmlEscape emailText}
      </incoming-email-content>

      <journal-context>
      #{_G.xmlEscape journalMatches ? 'No relevant journal entry found.'}
      </journal-context>

      <available-operations>
      - delete
      - move to {{destination}}
      </available-operations>

      <valid-move-destinations>
      #{_G.xmlEscape _G.renderMoveFolderChoicesLib _G.cachedMoveFolders}
      </valid-move-destinations>
      """

    result = await microagent.run { prompt }
    ref = if result.journal_id == 0 then 'Guess' else "Journal #{result.journal_id}"
    confidence = Number(result.confidence ? 0)
    output = { ref, confidence, ...result, ctx: microagent.ctx }
    _G.log 'microagent.result',
      name: 'recommendActionMicroagent'
      input: { emailText, journalMatches }
      output
      rawOutput: result
    , 'microagent'
    output
