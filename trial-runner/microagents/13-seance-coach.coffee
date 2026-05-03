import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.coffee'

###
 Seance-coach microagent.

 Runs inside the tight verification loop in seance.coffee.  Given the email,
 the correct answer, the current trial rationale, and a history of previously
 attempted rationale candidates that still produced the wrong answer, it
 proposes the next candidate backprop_rationale to test.

 The agent accumulates context across loop iterations via preloadedMessages so
 each attempt builds on the reasoning from prior failures.
###
_G.seanceCoachMicroagent = (emailText, correctAnswer, trialRationale, previousAttempts, existingMessages) ->
  _G.traceStep '🔄', 'Seance coach proposing rationale', ->
    microagent = await Agent.factory
      model: 'copilot:claude-sonnet-4.6'
      system_prompt: """
        You are a coach refining a one-to-three sentence rationale that will be
        injected as journal context into an email triage agent's recommend stage.
        Your goal: produce a rationale that reliably causes the agent to choose
        the correct action for this email.
        Propose a candidate rationale, then the system will test it.
        If it fails, you will be shown the wrong answer and asked to revise.
        """
      output_tool:
        description: 'Propose a candidate backprop_rationale to test'
        parameters:
          reasoning:
            type: 'string'
            description: 'Brief explanation of why you think this rationale will work (≤20 words)'
          backprop_rationale:
            type: 'string'
            description: 'The candidate rationale (1-3 sentences). Must be specific enough to steer the agent to the correct action without naming the answer explicitly.'
        required: ['reasoning', 'backprop_rationale']

    attemptsSection = if previousAttempts.length
      lines = previousAttempts.map (a, i) ->
        "Attempt #{i + 1}: rationale=#{JSON.stringify a.rationale} → agent chose #{JSON.stringify a.got}"
      "\n<previous-failed-attempts>\n#{lines.join '\n'}\n</previous-failed-attempts>"
    else ''

    prompt = """
      <email-content>
      #{_G.xmlEscape emailText}
      </email-content>

      <correct-answer>#{_G.xmlEscape correctAnswer}</correct-answer>

      <current-trial-rationale>
      #{_G.xmlEscape trialRationale}
      </current-trial-rationale>#{attemptsSection}

      Propose a revised rationale that will cause the agent to choose: #{JSON.stringify correctAnswer}
      """

    result = await microagent.run
      messages: existingMessages  # accumulate context across iterations
      prompt: prompt

    _G.log 'microagent.result',
      name: 'seanceCoachMicroagent'
      input: { correctAnswer, attempts: previousAttempts.length }
      output: result
    , 'microagent'

    { result, ctx: microagent.ctx }
