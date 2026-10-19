# My Agents

This repository is a collection of task-focused agents and the supporting libraries they use to make decisions, call tools, and automate workflows.

The best thing about this construction is it never runs out of context and can work forever,
for just the cost of electricity to my GPU.

## Versions

Each generation of this project lives on its own branch:

| Branch | Stack | What it is |
| --- | --- | --- |
| `main` | Bun + CoffeeScript | the original `personal-email` agent (this branch) |
| [`v2`](https://github.com/mikesmullin/agl-agents/tree/v2) | Bun + CoffeeScript | adds the `trial-runner` self-improving harness and the `email-trainer` card-feed UI |
| [`v3`](https://github.com/mikesmullin/agl-agents/tree/v3) | Bun + CoffeeScript | adds the Gmail/Outlook-style `email-trainer-2` UI |
| [`v4`](https://github.com/mikesmullin/agl-agents/tree/v4) | **Rust** (server + wasm on rustex1-kit) | **`mail1`** — rewrite of the agent and of the v3 UI: a SQLite mirror of the mailbox, an outbox of queued mutations pushed to Gmail in batches, `rules/*.yaml` x a local System One model for routing, `phrasers/*.yaml` x a local text LLM for rewording |

`v4` is an orphan branch: it shares no history with `main`, because the whole
stack changed. Its rule set was derived from the journal and presentation
databases that the agent on this branch writes into `personal-email/db/`.

## Available Agents

| Agent | Purpose |
| --- | --- |
| [personal-email](./personal-email/) | Interactive Gmail triage agent that pulls unread mail, summarizes it, recommends actions, executes email mutations, recalls memo context, and applies formattng preferences. |

## Repository Layout

- [personal-email/](./personal-email/) contains the current agent implementation, its microagents, and its local data.
- [lib/](./lib/) contains shared runtime helpers such as tracing, shell execution, Gmail integration, memo integration, and voice playback.

## Dependencies

- 👼 [agl-ai](https://github.com/mikesmullin/agl#readme) agent framework
- ✉️ [google-mail](https://github.com/mikesmullin/google-mail/) for Gmail integration
- 🧠 [memo](https://github.com/mikesmullin/memo/) for long-term memory
- 🤖 [MICROAGENT.md](https://github.com/mikesmullin/agl/blob/main/docs/MICROAGENT.md) maximizing effectiveness
- 🗣️ [voice](https://github.com/mikesmullin/voice) for TTS playback