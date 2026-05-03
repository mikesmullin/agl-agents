import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

###
 Coach summary microagent.

 Runs once per trial run after all row analyses are complete.
 Receives the full table of results and produces:
   score         – "N/M = X% = Grade Y"
   encouragement – ≤2 optimistic sentences aimed at the next generation of coach
###
_G.coachSummaryMicroagent = (rows) ->
  _G.traceStep '📊', 'Coach summary', ->
    microagent = await Agent.factory
      system_prompt: """
        You are a coach reviewing a trial run of an AI email triage agent.
        The trial measures whether the agent recommends the correct email action
        given a short rationale as context instead of a full journal database.
        Based on the results table, produce an overall score and an encouraging
        note that helps the next generation of coach focus on what matters most.
        """
      output_tool:
        description: 'Return overall trial assessment'
        parameters:
          score:
            type: 'string'
            description: 'Score in the format "(passed/total) XX% = Grade Y" where grade is A(90-100), B(80-89), C(70-79), D(60-69), F(<60).'
          encouragement:
            type: 'string'
            description: '≤2 sentences addressed to the next coach iteration. Optimistic, specific, forward-looking. "Assume the sale" — express faith that the goal is achievable.'
        required: ['score', 'encouragement']

    tableRows = rows.map (r) ->
      status = if r.passed then 'PASS' else 'FAIL'
      "| #{r.entityId} | #{status} | #{r.given} | #{r.correct} | #{r.feedback} |"

    prompt = """
      <results-table>
      | Entity | Result | Given Answer | Correct Answer | Feedback |
      |--------|--------|--------------|----------------|----------|
      #{tableRows.join '\n'}
      </results-table>

      <pass-count>#{rows.filter((r) -> r.passed).length}</pass-count>
      <total-count>#{rows.length}</total-count>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'coachSummaryMicroagent'
      input: { total: rows.length, passed: rows.filter((r) -> r.passed).length }
      output: result
    , 'microagent'
    result
