import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.buildJournalEntryMicroagent = (emailText, instruction, executionOutcome) ->
  _G.traceStep '📝', 'Building journal entry', ->
    microagent = await Agent.factory
      system_prompt: """
        Create a compact journal record describing how we handled one email-content.
        """
      output_tool:
        parameters:
          summary:
            type: 'string'
            description: 'Short description of the email content.'
          keywords:
            type: 'string'
            description: 'One comma-separated keyword list combining high-signal identifiers and useful topic labels for future deterministic matching. Include brand names, proper nouns, domain fragments, subject terms, and category labels.'
          action_taken:
            type: 'string'
            description: 'What action was taken.'
          factors:
            type: 'string'
            description: """Decision factors as concise phrases. Factors must preserve user rationale and decision criteria from instruction. If instruction includes numeric ranges/thresholds, include them explicitly in factors. If instruction includes conditional follow-up behavior, include that in factors. Keep factors concise and semicolon-separated."""
          sender_email:
            type: 'string'
            description: 'Full from-address extracted from the email, normalized to lowercase. Empty string if not present.'
          sender_offers:
            type: 'string'
            description: 'One sentence: what the sender provides or sells (≤15 words). Use "none" if not applicable.'
          sender_expects:
            type: 'string'
            description: 'One sentence: the call to action the sender wants the reader to take (≤15 words).'
          reader_value:
            type: 'string'
            description: 'One sentence: the potential benefit or value to the reader, or "none" (≤15 words).'
          match_criteria:
            type: 'string'
            description: 'Comma-separated identifiers that would reliably re-match this email type (e.g. sender domain, subject keywords, body keywords).'
          rule:
            type: 'string'
            description: 'Generalized future-action rule derived from this instruction: what to do with similar emails in the future.'
        required: [
          'summary'
          'keywords'
          'action_taken'
          'factors'
          'sender_email'
          'sender_offers'
          'sender_expects'
          'reader_value'
          'match_criteria'
          'rule'
        ]

    prompt = """
      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>

      <user-instruction>
      #{_G.xmlEscape instruction}
      </user-instruction>

      <execution-outcome>
      #{_G.xmlEscape String(executionOutcome)}
      </execution-outcome>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'buildJournalEntryMicroagent'
      input: { emailText, instruction, executionOutcome }
      output: result
    , 'microagent'
    result
