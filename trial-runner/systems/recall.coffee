import { _G } from '../../lib/globals.coffee'

###
 Mock recall system for trial mode.

 Instead of calling the memo database (which has imperfect recall), this
 system directly injects entity._trial.trial_rationale as the journal context
 fed to the stage-6 recommend microagent.

 This simulates *perfect* journal recall so that trial-runner can focus its
 learning signal on the recommend microagent's reasoning — not on the search
 strategy.  The rationale text is intentionally terse (1-2 sentences) and is
 the lever coach tweaks across trial generations.
###
export recallSystem = ->
  entities = _G.World.Entity__find (e) -> e.fingerprint? and not e.recall?
  for entity in entities
    _G.currentEntityId = entity.id

    trialRationale = String(entity._trial?.trial_rationale or '').trim() or
      'No relevant journal entry found.'

    await _G.Entity.patch entity, 'recall',
      journalContext: trialRationale
      presentationCandidate:
        has_formatting_instructions: false
        applies_if: ''
        formatting_instructions: ''
      usePresentationPreferences: false
