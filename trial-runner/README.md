# trial-runner

`trial-runner` is a self-improving test harness for agl-agents.  It runs the target agent (currently `personal-email`) against a set of archived training entities, grades the results, and produces a report card that a coach LLM uses to refine the rationale fed to the agent — all without touching real Gmail or the memo database.

## Quick start

```bash
# From the repository root — run after personal-email has archived ≥1 completed entity
bun trial-runner/agent.coffee
```

Each invocation is one **trial run** (one generation).  Results land in:

```
personal-email/db/_archive/trial/
  001/
    entities/          ← stripped trial entity YAMLs (one per training email)
    report-card.md     ← the scored report card
```

## How it works

### Training data

Trial-runner draws its training examples exclusively from `personal-email/db/_archive/`.  Only entities where `operator.command == 'proceed'` are included — this is the **reward signal**: the operator validated the agent's recommendation, so `recommendation.operations` from that run is the ground truth.

### Pipeline (one pass per trial run)

| # | System | Mode |
|---|--------|------|
| 1 | **page** | no-op (no Gmail pull) |
| 2 | **load** | mocked: parses `origin.raw` from the trial entity instead of calling `google-email` |
| 3 | **fingerprint** | real LLM inference (same microagent as personal-email) |
| 4 | **seed-journal** | writes `_trial.trial_rationale` as structured journal entries into an isolated per-trial memo database (`trial/NNN/journal`) |
| 5 | **recall** | **real** hybrid vector+keyword+sender search against the trial journal — same system used by personal-email in production |
| 6 | **summarize** | real LLM inference |
| 7 | **recommend** | real LLM inference; also captures the full context window in `retrospective.stage_6_context` |
| 8 | **display** | deterministic |
| 9 | **operator** | **mocked gate**: compares `recommendation.operations` vs `_trial.correct_answer`; marks PASS/FAIL |
| 10 | **seance** | for FAIL entities: replays the stage-6 context window and asks the model why it chose wrong |
| 11 | **report** | for FAIL entities: runs coach LLM for feedback + new `backprop_rationale`; PASS entities carry forward existing rationale without an LLM call; writes `report-card.md` |

### How recall evolved

Early generations used a **mocked recall** step that injected `_trial.trial_rationale` directly as `recall.journalContext`, simulating perfect recall.  This isolated the core hypothesis: *given perfect recall, does the rationale text cause the recommend microagent to choose correctly?*  Proving that first let us iterate fast on rationale quality without being confounded by search quality.

Once rationale quality was proven at scale (trials 007–010 reached 100% Grade A with mocked recall), we introduced the **seed-journal + real recall** approach (trial 013 onward).  The coach's `backprop_rationale` is now written as structured journal entries into an isolated per-trial memo store; the real `recallSystem` then searches that store using the same hybrid vector+keyword+sender strategy as production.  Trials 013–014 confirmed the system generalises: the agent scores **92–100% Grade A** even when recall is no longer hand-delivered — demonstrating that the self-learning loop produces rationale phrasing that is both *correct* and *searchable*.

### The learning loop (generations)

```
Generation 0
  trial_rationale = operator.rationale (or recommendation.rationale from archive)
  → trial run → report card
    → coach writes backprop_rationale per entity
      → next run seeds trial_rationale from backprop_rationale
        → Generation 1 ...
```

Coach tweaks `trial_rationale` (the injected journal context) after each run to push the agent toward consistent correct answers.

### The seance

When an entity fails the operator gate, trial-runner "seances" the recommend microagent by restoring the exact context window it held when it chose wrong and appending:

> *The correct answer was "X" but you chose "Y". Why? How could the `<journal-context>` have been phrased differently to steer you toward the correct answer?*

The assistant's introspective answer is passed to the coach-row microagent alongside the regular inputs, giving coach richer signal for writing the next `backprop_rationale`.

## Report card format

See [EXAMPLE_REPORT.md](./EXAMPLE_REPORT.md) for a real report card produced by trial run 002 (12 entities, 8/12 = 67% Grade D on generation 0).

```markdown
# Trial Run 001 — Report Card

> Agent: personal-email/README.md

## Results

| Entity | Result | Given Answer | Correct Answer | Trial Rationale | Original Rationale | Feedback | Backprop Rationale |
...

## Summary

**Score:** (8/10) 80% = Grade B
**Encouragement:** ...
```

## Microagents

- [11-coach-row.coffee](./microagents/11-coach-row.coffee) — per-entity coaching: feedback (continue/start/stop) + backprop_rationale
- [12-coach-summary.coffee](./microagents/12-coach-summary.coffee) — overall score + encouragement for the next generation

## Entity `_trial` fields

Each trial entity carries a `_trial` component that persists across the pipeline run:

| Field | Description |
|-------|-------------|
| `correct_answer` | `recommendation.operations` from the original live run (the ground truth) |
| `original_rationale` | `recommendation.rationale` from the original live run |
| `trial_rationale` | Current-generation rationale injected as mock journal context; updated by coach each generation |
| `backprop_rationale` | Coach's revised rationale written after the report; becomes `trial_rationale` next generation |

## Configuration

Trial-runner inherits `_G.MODEL` from [lib/globals.coffee](../lib/globals.coffee).  No additional config file is needed for the first run.
