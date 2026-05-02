import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.mjs'

_G.journalEntryRelevanceFilterMicroagent = (emailText, instruction, patternSummary, journalEntryText, operatorHint = '') ->
  _G.traceStep '🔍', 'Filtering journal entry relevance', ->
    microagent = await Agent.factory
      system_prompt: """
        You decide whether a journal-entry is relevant to a detected pattern in the context of 
        the current email and user instruction.
        Weigh same vendor/sender matches significantly higher than different-vendor entries — 
        a journal rule about the same company or sender domain is almost always more relevant 
        than a topically similar rule from a different source.
        If an operator-hint is provided, treat it as the primary relevance criterion and 
        prioritise entries that match it above all other factors.
        """
      output_tool:
        parameters:
          relevant:
            type: 'boolean'
            description: 'True if journal-entry is related to the detected pattern and likely covers similar emails to the current one. False otherwise.'
          confidence:
            type: 'integer'
            description: 'Confidence score 0–100 that this journal entry is relevant to the pattern.'
        required: ['relevant', 'confidence']

    prompt = """
      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>

      <user-instruction>
      #{_G.xmlEscape instruction}
      </user-instruction>

      <pattern-summary>
      #{_G.xmlEscape patternSummary}
      </pattern-summary>
      #{if operatorHint then """
      <operator-hint>
      #{_G.xmlEscape operatorHint}
      </operator-hint>
      """ else ''}
      <journal-entry>
      #{_G.xmlEscape journalEntryText}
      </journal-entry>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'journalEntryRelevanceFilterMicroagent'
      output: result
    , 'microagent'
    result
