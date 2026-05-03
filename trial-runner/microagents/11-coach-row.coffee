import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

###
 Coach row-analysis microagent.

 Runs once per trial entity.  Given the given vs. correct answers and the
 rationale texts, it produces:
   feedback          – continue/start/stop coaching note for this entity
   backprop_rationale – revised trial_rationale to use in the next generation
###
_G.coachRowMicroagent = (entityId, given, correct, trialRationale, originalRationale, seanceExplanation) ->
  _G.traceStep '🎓', "Coach row #{entityId}", ->
    microagent = await Agent.factory
      model: 'copilot:claude-sonnet-4.6'
      system_prompt: """
        You are a coach improving an AI email triage agent's recommendation accuracy.
        For each email entity you receive the agent's answer, the correct answer, and
        the rationale text that was injected as journal context.
        Your job: diagnose *why* the agent got it right or wrong, then write a revised
        rationale that would reliably steer the agent toward the correct answer.
        """
      output_tool:
        description: 'Return your coaching analysis'
        parameters:
          feedback:
            type: 'string'
            description: 'Coaching note in continue/start/stop format. E.g. "CONTINUE: X. STOP: Y. START: Z." (≤40 words)'
          backprop_rationale:
            type: 'string'
            description: 'Revised trial_rationale text (1-3 sentences) to inject next generation. Must be concise, declarative, and directly actionable by the recommend microagent.'
        required: ['feedback', 'backprop_rationale']

    seanceSection = if seanceExplanation
      """

      <seance-explanation>
      #{_G.xmlEscape seanceExplanation}
      </seance-explanation>
      """
    else ''

    prompt = """
      <entity-id>#{entityId}</entity-id>

      <given-answer>#{_G.xmlEscape given}</given-answer>
      <correct-answer>#{_G.xmlEscape correct}</correct-answer>

      <trial-rationale>
      #{_G.xmlEscape trialRationale}
      </trial-rationale>

      <original-rationale>
      #{_G.xmlEscape originalRationale}
      </original-rationale>#{seanceSection}
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'coachRowMicroagent'
      input: { entityId, given, correct }
      output: result
    , 'microagent'
    result
