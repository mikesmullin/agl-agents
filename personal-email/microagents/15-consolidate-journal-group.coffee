import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

_G.consolidateJournalGroupMicroagent = (emailText, instruction, patternSummary, journalEntriesText, extraInstructions = '') ->
  _G.traceStep '📦', 'Consolidating journal group', ->
    microagent = await Agent.factory
      system_prompt: """
        You merge a set of related journal-entries into a single consolidated rule 
        that captures the shared pattern, without losing critical distinctions or conditional logic.
        """
      output_tool:
        parameters:
          consolidated_rule:
            type: 'string'
            description: 'A single generalized rule describing what to do with emails matching this pattern. Should capture any conditional branches (e.g. "delete unless X, in which case archive").'
          consolidated_match_criteria:
            type: 'string'
            description: 'Comma-separated identifiers that reliably identify emails covered by this consolidated rule (sender domains, subject keywords, body keywords, etc.).'
          consolidated_action:
            type: 'string'
            description: 'The primary recommended action for future matching emails (e.g. "delete", "mark as read + move to Newsletters").'
          consolidated_rationale:
            type: 'string'
            description: 'Concise semicolon-separated decision factors explaining why these entries were grouped and what the rule covers (≤50 words).'
          entry_ids_to_supersede:
            type: 'array'
            items: { type: 'integer' }
            description: 'IDs of existing journal entries that are fully covered by this consolidated rule and can be replaced.'
        required: ['consolidated_rule', 'consolidated_match_criteria', 'consolidated_action', 'consolidated_rationale', 'entry_ids_to_supersede']

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

      <journal-entries>
      #{_G.xmlEscape journalEntriesText}
      </journal-entries>

      <extra-instructions>
      #{_G.xmlEscape extraInstructions}
      </extra-instructions>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'consolidateJournalGroupMicroagent'
      output: result
    , 'microagent'
    result
