# mail1 — v4

The fourth generation of this project, and the first where the routing decision is
not made by a chat model.

`mail1` is a local Gmail triage app: a SQLite mirror of the mailbox, a YAML rule
set, a **System One decision model** that says what each email deserves, an outbox
of queued mutations that are pushed to Gmail in batches, and a browser UI — all in
Rust, front end and back.

## Lineage

| Branch | Stack | Decision mechanism |
| --- | --- | --- |
| `main` | Bun + CoffeeScript | one LLM per decision, structured output via `agl` tool calls |
| [`v2`](https://github.com/mikesmullin/agl-agents/tree/v2) | Bun + CoffeeScript | same, plus `trial-runner`: a coach LLM back-propagating rationale across generations |
| [`v3`](https://github.com/mikesmullin/agl-agents/tree/v3) | Bun + CoffeeScript | same, plus the Gmail-style `email-trainer-2` operator UI |
| **`v4`** | **Rust** (server + Rust→wasm front end) | **deterministic rules × a System One decision model** |

## What main / v2 / v3 were optimizing for

Those three generations were built on 👼 [agl](https://github.com/mikesmullin/agl)
(*Another Generative Language*) and its [microagent
contract](https://github.com/mikesmullin/agl/blob/main/docs/MICROAGENT.md):

- one question, one `Agent.factory`, one `run`, **one final output-tool call**;
- the answer arrives as a **typed, schema-validated tool call**, so deterministic
  code can consume it without parsing prose;
- deterministic code owns I/O, parsing, retries, persistence and side effects; the
  model owns only the subjective judgment;
- judgment is grounded by **recall** — hybrid vector + keyword + sender search over
  a journal memo database — and improved by `trial-runner`, which treats the
  operator's `proceed` as the reward signal, grades each generation, and has a coach
  LLM rewrite the injected rationale until the agent answers consistently.

That worked, and it is where the training data came from: the journal and
presentation databases this project's earlier agents wrote into
`personal-email/db/` (≈1.5k journal entries, ≈1.7k presentation preferences). The
`rules/*.yaml` on this branch were derived from exactly those records.

But every decision still cost a full chat completion: a long system prompt,
retrieved context, generated JSON, a parse, a retry, and a self-reported
confidence number that the model invented rather than measured. Latency was
seconds, cost was tokens, and "how sure are you?" was prose.

## What is new in v4: a System One decision model

v4's major move is to replace the chat model in the decision slot with a
**System One** (System 1) model — a *decision model*, which returns typed answers
and calibrated probabilities instead of generated text. You give it a **state** and
**typed questions**; it returns the chosen option, a probability for every option,
and a confidence value. Jev (TypeSafe AI) was the first; open-weights ones you can
run on your own GPU exist now. See [systemonemodels.org](https://systemonemodels.org).

Every call is `POST /v1/systemone` with `{ state, questions }` → `{ answers }`, and
every question is one of three shapes — all three are used by this app's rules:

| Type | Question | Answer |
| --- | --- | --- |
| `choice` | pick one of up to 255 options you define | winning option + a probability per option + confidence |
| `score` | rate the state against 2–10 ordered levels you write | probability-weighted mean, may land between levels |
| `noul` | yes/no | one number, 0…1 |

Why that is the next level for *this* workload:

- **Fast.** A decision is tens to a few hundred milliseconds, not a multi-second
  completion. Triaging a mailbox becomes a batch job rather than a conversation.
- **Accurate in a measurable way.** Probabilities are calibrated outputs, so a
  threshold is a number the code branches on — not a 0–100 the model was asked to
  improvise (v3's `recommendation.confidence`).
- **Nothing to parse, nothing to hallucinate.** The answer *is* the action. The
  option names are the app's action vocabulary (`archive`, `delete`, `skip`,
  `move:Expenses`), so 255 options comfortably hold every folder in the mailbox.
- **Fan-out.** One request per email carries the prompts of *every* rule that
  matched, and parallel questions in a single request run in parallel — so adding
  rules adds coverage, not latency.
- **Scale.** Thousands of emails, one GPU, no per-decision API bill.

### The split: deterministic first, model second

The rules decide **whether to ask and what to ask**; the model decides **which
action this email deserves**. `match:` / `unless:` are regexes and label tests
evaluated in Rust — a fact, reviewable, free, and immune to drift. Only the emails
that survive get a question, phrased for the slice of mail that rule describes.

```yaml
# illustrative; the real rule set lives in rules/ (gitignored)
name: card-transaction-alert
match:  { sender: .+@mybank\.com, snippet: transaction|purchase }
prompts:
  - id: transaction-alert
    type: choice
    instructions: Is this a card purchase alert?
    criteria:
      move:Expenses: You made a $ transaction with a merchant
      skip: statement, payment reminder, survey or privacy notice
```

The second model slot is a text LLM, used only for **presentation**: `phrasers/*.yaml`
match with the same syntax but ask for a rewording, and the reply replaces the
row's subject and summary. One GPU, two slots (`:1234` text, `:1237` decision), so
the swap is manual and a missing model is reported per email instead of silently
skipped.

## Why Rust on both ends

The stack change is not a preference, it is a decision about **who edits the code**.

- **The compiler is the reviewer.** A typed boundary — a struct on the server, a
  `serde_json` shape on the wire, a store field in wasm — fails loudly when an LLM
  gets it wrong. An untyped CoffeeScript object graph fails silently, and the
  failure surfaces three stages later as an empty card in the UI.
- **One language across the whole system.** The route handler, the SQLite row, the
  queue, the JSON, the store and the markup are all Rust, so a model can hold a
  change in its head without translating between a JS front end and a JS back end.
- **It scales in SLOC.** This branch is ~4.4k lines of server, ~3.7k lines of app
  and a ~4.5k-line kit on top of a ~2.3k-line dev server — a size where the missing
  type information, not the missing feature, is what breaks an edit.
- **The kit enforces the conventions.** `rustex1-kit` carries the linter
  (`lintReport()`), the template dumper (`dumpTemplate()`), the hot-swap runtime and
  the shared widgets, so a generated page looks like the other pages and a bad one
  reports itself.
- **Hot reload keeps the model in the loop.** The dev server watches sources, rebuilds
  wasm, and hot-swaps the m-js components — state survives, so an edit is judged on
  the live inbox rather than a mock.

## Layout

| Path | What it is |
| --- | --- |
| `server/src/main.rs` | boot: SQLite handle, optional Gmail client, label cache, route table |
| `server/src/gmail.rs` | from-scratch Gmail REST client — OAuth2 refresh token + `ureq`, no googleapis SDK (`list`, `get`, `modify`, `batchModify`, `trash`, `delete`, `labels`) |
| `server/src/db.rs` | the mirror: `message`, `mutation` (outbox), `label`, `phrase`, `backlog`, `meta` |
| `server/src/sync.rs` | **pull** (paged list + N workers over `messages.get`) and **push** (outbox drained in order, grouped into `batchModify`) |
| `server/src/rules.rs` | rule YAML, deterministic match, optional rendered-screenshot vision, the `/v1/systemone` client |
| `server/src/phrasers.rs` | presentation rules, the chat-completions client, stored rewordings |
| `server/src/routes.rs` | the JSON API the front end talks to |
| `app/src/` | Rust→wasm UI on `rustex1-kit`: `pages` (markup), `store` (state + network), `keys` (hotkeys) |
| `scripts/` | mock System One and fake text LLM (real wire shape, canned judgment) + a Phosphor glyph checker |
| `vendor/rustex1/` | the kit + dev server, committed as plain files (upstream is local-only, so a submodule URL would not resolve) |

## Run

```bash
cargo run -p mail1-server          # http://localhost:3030  (port from rustex1.json)
```

The dev server builds the wasm half, serves `web/`, and hot-reloads both sides.
Gmail is optional at boot: without credentials the mirror is still browsable and the
outbox still queues, and every Gmail route answers 503 with the reason.

| Env | Default | Meaning |
| --- | --- | --- |
| `SYSTEMONE_URL` | `http://127.0.0.1:1237` | the decision-model slot (`/v1/systemone`) |
| `LLM_URL` | `http://127.0.0.1:1234` | the text-LLM slot (`/v1/chat/completions`) |
| `GMAIL_AUTH` | `credentials.json` + `token.json`, or `gmail.json` | OAuth *authorized_user* file; the refresh token never leaves the machine |

No GPU? Exercise both paths end to end with the mocks:

```bash
python3 scripts/mock-systemone.py --port 1237   # word overlap, not a model
python3 scripts/fake-llm.py --port 1234         # canned SUBJECT:/SUMMARY:
```

Keyboard: `j`/`k` move, `space`/`x` select, `enter` open, `a` select all, `q` read,
`r` run rules, `z` queue the recommendation, `y` star, `p` pull, `u` push, `/` search,
`esc` back out one level.

## Not in git, on purpose

`rules/`, `phrasers/` and `data/` are gitignored. A rule set and a mirror of somebody's
mailbox are personal artifacts, not source — the same reason `personal-email/db/` is
gitignored on the older branches. `target/`, `web/pkg/`, `tools/` and `.venv/` are
build and serving output.

## Links

- [System One Models](https://systemonemodels.org) — decision models, the three
  question types, hosted and open-weights options
- 👼 [agl](https://github.com/mikesmullin/agl) — the framework the earlier generations used
- [v3](https://github.com/mikesmullin/agl-agents/tree/v3) — the CoffeeScript agent + `email-trainer-2` UI this branch replaces
