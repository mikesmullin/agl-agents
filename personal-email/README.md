# personal-email

`personal-email` is a headless, unattended email triage agent for Gmail. On each loop iteration it pulls all unread messages within a rolling 14-day window, processes them in parallel through an ECS pipeline, and persists every decision to a per-entity YAML file on disk. The operator reviews and responds by editing those YAML files directly — no interactive prompt.

## Run From The Command Line

From the repository root:

```bash
bun personal-email/agent.coffee
```

The loop runs indefinitely (10-second sleep between iterations). Press Ctrl+C once to request a graceful finish after the current iteration; press it again to force-quit immediately.

## Architecture

The agent uses an **Entity–Component–System (ECS)** model:

- **World** (`models/world.coffee`) — in-memory registry of all live entities (one per email). Provides `Entity__find(filterFn)` used by every system to query the entities it should act on.
- **Entity** (`models/entity.coffee`) — static class (never instantiated) that owns all disk I/O. Every read re-loads from disk so operator edits are always picked up on the next iteration. Provides `load`, `save`, `patch`, `delete`, `log`, and `traceStart`.
- **Components** — named fields on the entity object (e.g. `envelope`, `content`, `fingerprint`, `operator_input`). Component *presence* is the gate: a system processes an entity only when its required components are present (and its own output component is absent).
- **Systems** — pure async functions in `systems/`. Each queries World, iterates matching entities, and patches components.

## What It Does

- Calls `google-email pull --since "14 days ago"` on every loop iteration; uses the `transitions[]` array in the YAML output to detect new, deleted, and moved emails and update World and disk accordingly.
- Loads email envelope and cleaned body content into entity state.
- Fingerprints each email (keywords, intent) via microagent.
- Recalls relevant journal history and presentation preferences from memo databases.
- Summarizes the email and recommends an action (with confidence score).
- Writes an email card to the entity `log[]` for the operator to review.
- Gates on `operator_input.instruction` — written by the agent as `null`; the operator fills it in to unlock the next stage.
- Executes the Gmail mutation and persists a journal entry.
- Generates a mutation plan, then gates on `apply.approved: true` before applying.

## Processing Loop

Each iteration executes all systems in sequence against all live entities in World. Systems skip entities that don't satisfy their gate predicate. All component paths refer to fields in `personal-email/db/entities/<id>.yaml`.

> ✍️ = must be filled in by the operator by editing the entity YAML file directly on disk

| # | System | Inputs | Outputs |
|---|--------|--------|---------|
| 1 | **page** — pull emails; sync World from transitions | — | new entities loaded into World;<br>deleted/archived entities removed from World + disk |
| 2 | **load** — fetch envelope and body | `id` | `origin.raw`<br>`envelope.from`<br>`envelope.subject`<br>`envelope.date`<br>`envelope.senderEmail`<br>`content.body` |
| 3 | **fingerprint** — extract keywords and intent | `content.body` | `fingerprint.keywords`<br>`fingerprint.sender_offers`<br>`fingerprint.sender_expects`<br>`fingerprint.reader_value` |
| 4 | **recall** — retrieve journal and presentation context | `content.body`<br>`fingerprint.keywords`<br>`fingerprint.sender_offers`<br>`fingerprint.sender_expects`<br>`fingerprint.reader_value`<br>`envelope.senderEmail` | `recall.journalContext`<br>`recall.presentationCandidate`<br>`recall.usePresentationPreferences` |
| 5 | **summarize** — generate headline and summary | `content.body`<br>`recall.usePresentationPreferences`<br>`recall.presentationCandidate.formatting_instructions` | `summary.headline`<br>`summary.text` |
| 6 | **recommend** — choose action and confidence | `content.body`<br>`recall.journalContext` | `recommendation.label`<br>`recommendation.confidence`<br>`recommendation.journal_id`<br>`recommendation.ref`<br>`recommendation.operations`<br>`recommendation.rationale`<br>`retrospective.stage_6_context` (context window snapshot for seance)<br>`retrospective.stage_6_model` |
| 7 | **display** — write email card to entity log | `envelope.from`<br>`envelope.subject`<br>`envelope.date`<br>`summary.headline`<br>`summary.text`<br>`recommendation.label`<br>`recommendation.confidence`<br>`recall.usePresentationPreferences`<br>`recall.presentationCandidate.formatting_instructions` | `log[]` |
| 8 | **operator** (stage 1) — open human input gate | `recommendation.label` | `operator_input.instruction: null`<br>`operator_input.operation: null`<br>`operator_input.rationale: null`<br>`operator_input.recommendation` |
| 9 | **operator** (stage 2) — process instruction | ✍️ `operator_input.instruction` OR `operator_input.operation`<br>`operator_input.rationale` (optional)<br>`content.body`<br>`recommendation.label`<br>`recommendation.journal_id` | `operator.command`<br>`operator.instruction`<br>`operator.instructionOrRecommendation`<br>`operator.rationale`<br>— or on memo/question:<br>`operator_input.last_result`<br>`operator_input.last_answer`<br>`operator_input.instruction` reset to `null` |
| 10 | **refresh** — clear components for re-processing | `operator.command` (`refresh` or `reload`) | clears `operator_input`<br>`fingerprint`<br>`recall`<br>`summary`<br>`recommendation`<br>`operator`<br>`execution`<br>`journal` |
| 11 | **reload** — hot-reload microagents, then refresh | `operator.command` (`reload`) | reloads all microagent modules; same clears as **refresh** |
| 12 | **execute** — run Gmail instruction | `operator.instruction`<br>`operator.command` | `execution.success`<br>`execution.summary`<br>`execution.instruction`<br>`log[]` |
| 13 | **journal** — persist outcome to memo database | `content.body`<br>`operator.instructionOrRecommendation`<br>`operator.command`<br>`execution.summary`<br>`recommendation.journal_id` | `journal.summary`<br>`journal.keywords`<br>`journal.action_taken`<br>`journal.factors`<br>`journal.sender_email`<br>`journal.sender_offers`<br>`journal.sender_expects`<br>`journal.reader_value`<br>`journal.match_criteria`<br>`journal.rule` |
| 14 | **plan** — generate pending mutation plan | `journal` (presence gate) | `plan.success`<br>`plan.text`<br>`plan.planned_at`<br>`log[]` |
| 15 | **apply** — approve and execute mutations | ✍️ `apply.approved: true`<br>(agent writes `null`; operator sets `true`) | `apply.success`<br>`apply.output`<br>`apply.applied_at`<br>`log[]` |
| 16 | **clean** — archive completed entities | `apply.success: true`<br>`apply.applied_at` (≥ 1 min ago) | entity YAML moved to `db/_archive/`;<br>entity removed from World |

## Operation

The agent runs headlessly — no interactive prompt. To respond, edit the entity YAML in `personal-email/db/entities/<id>.yaml` directly. The agent picks up changes on the next loop iteration (every ~10 seconds).

**`operator_input.instruction`** (legacy free-text) or **`operator_input.operation`** + **`operator_input.rationale`** (structured) — set one of these to unlock the next stage.

- Use `operation` + `rationale` when you want to provide explicit training signal. The `rationale` (1-2 sentences) becomes the initial `trial_rationale` seed for trial-runner and is stored on `operator.rationale` for analysis.
- Use `instruction` for ad-hoc commands (memo operations, questions, custom Gmail instructions).

**`operator_input.instruction`** — set this field to one of:

- `proceed` or `p` — apply the recommended action.
- `skip` — skip this email without executing anything.
- `quit` — stop the agent loop after the current iteration finishes.
- `refresh` — clear all computed components and re-process from scratch.
- `reload` — hot-reload microagent modules from disk, then re-process.
- A memo command (e.g. `journal ...`) — operate on the journal or presentation memo databases; `instruction` is reset to `null` and `last_result` is written back so you can chain commands.
- A natural-language question — answered from the email content; `instruction` reset to `null`, answer written to `last_answer`.
- Any other text — treated as a custom Gmail instruction passed directly to the execute stage.

**`apply.approved`** — after the plan stage writes the proposed Gmail mutation plan to `plan.text`, the agent writes `apply.approved: null`. Set it to `true` to authorize the agent to call `google-email apply`.

## Key Files

- [agent.coffee](./agent.coffee) — main loop.
- [models/world.coffee](./models/world.coffee) — in-memory entity registry.
- [models/entity.coffee](./models/entity.coffee) — disk-backed entity persistence (load, save, patch, delete, log, trace).
- [systems/](./systems/) — one file per pipeline stage.
- [microagents/](./microagents/) — single-purpose model-driven decision units.
- [db/journal.memo](./db/journal.memo) — journal memo database (action history).
- [db/presentation.memo](./db/presentation.memo) — presentation preferences memo database.
- [db/entities/](./db/entities/) — one YAML file per live email entity.

## Microagents

For a full definition of the microagent pattern see [docs/ECS.md](./docs/ECS.md).

- [00-fingerprint-email.coffee](./microagents/00-fingerprint-email.coffee) — extracts keywords and intent from email body.
- [01-recommend-action.coffee](./microagents/01-recommend-action.coffee) — recommends what to do with the email.
- [02-contains-question.coffee](./microagents/02-contains-question.coffee) — decides whether an operator instruction is a question.
- [04-answer-question-from-email.coffee](./microagents/04-answer-question-from-email.coffee) — answers operator questions from email content.
- [05-summarize-email.coffee](./microagents/05-summarize-email.coffee) — generates headline and summary text.
- [06-presentation-rule-relevance.coffee](./microagents/06-presentation-rule-relevance.coffee) — checks whether saved formatting preferences apply.
- [07-execute-memo-instruction.coffee](./microagents/07-execute-memo-instruction.coffee) — handles memo database instructions.
- [08-execute-instruction.coffee](./microagents/08-execute-instruction.coffee) — executes Gmail actions.
- [09-build-journal-entry.coffee](./microagents/09-build-journal-entry.coffee) — turns the outcome into a structured journal record.
- [10-build-presentation-entry.coffee](./microagents/10-build-presentation-entry.coffee) — extracts reusable formatting preferences.

## Configuration & Data

- Root [config.yaml.example](../config.yaml.example) — shows all supported config keys including `email.provider` and `journal.confidence_threshold`.
- [config.yaml](./config.yaml) — local agent config (gitignored).
- [db/](./db/) — journal and presentation memo state.
- [db/entities/](./db/entities/) — live entity YAML files (one per email currently in the pipeline).
- Shared helpers live in [../lib/](../lib/).
