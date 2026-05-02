import { _G } from '../../lib/globals.coffee'

export displaySystem = (confidenceThreshold) ->
  entities = _G.World.Entity__find (e) -> e.recommendation? and not e.operator_input? and not e.operator?
  for entity in entities
    _G.currentEntityId = entity.id
    { envelope, summary, recommendation, recall } = entity
    confidence = recommendation.confidence
    lowConfidence = confidence < confidenceThreshold

    presentationText = _G.optionalText recall.usePresentationPreferences, recall.presentationCandidate?.formatting_instructions

    await _G.traceStep '📨', 'Writing email card', ->
      _G.Entity.log entity, """
        ========== NEXT EMAIL ==========
        From: #{envelope.from}
        Subj: #{envelope.subject}
        Date: #{envelope.date}
        🗣️ #{summary.text}#{if presentationText then "\n\nApplied preferences:\n#{presentationText}" else ''}

        Recommended action:
        #{String(recommendation.label or '')} (confidence: #{confidence}%#{if lowConfidence then ' ⚠️ uncertain' else ''})
        ===============================
        """
