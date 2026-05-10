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
              Which single operation do you recommend?
              You MUST output exactly one of these forms — nothing else:
                delete
                archive
                skip
                move to <FolderName>
              where <FolderName> is the exact folder name from the valid-move-destinations list (text before ' — ').
              Do NOT add extra words, punctuation, or description. Output only the operation string.
              If journal-context recommends delete, output exactly: delete
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
      delete
      archive
      skip
      move to <FolderName>   (example: "move to Expenses" or "move to Newsletters")
      </available-operations>

      IMPORTANT: For move operations you MUST include the "move to " prefix.
      Never output just a folder name alone.

      <valid-move-destinations>
      #{_G.xmlEscape _G.renderMoveFolderChoicesLib _G.cachedMoveFolders}
      </valid-move-destinations>
      """

    result = await microagent.run { prompt }

    # Normalise operations: if the LLM returned a bare folder name without the
    # required "move to " prefix, add it. This guards against models that copy
    # the folder name directly from the destination list.
    ops = String(result.operations or '').trim()
    knownSimple = new Set ['delete', 'archive', 'skip']
    unless knownSimple.has ops.toLowerCase() or ops.toLowerCase().startsWith 'move to '
      # Check if it matches a known folder name (case-insensitive)
      folderNames = Object.keys(_G.cachedMoveFolders or {})
      matched = folderNames.find (f) -> f.toLowerCase() is ops.toLowerCase()
      result.operations = if matched then "move to #{matched}" else ops

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
