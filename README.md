# My Agents

This repository is a collection of task-focused agents and the supporting libraries they use to make decisions, call tools, and automate workflows.

The best thing about this construction is it never runs out of context and can work forever,
for just the cost of electricity to my GPU.

## Available Agents

| Agent | Purpose |
| --- | --- |
| [personal-email](./personal-email/) | Interactive Gmail triage agent that pulls unread mail, summarizes it, recommends actions, executes email mutations, recalls memo context, and applies formattng preferences. |
| [trial-runner](./trial-runner/) | Self-improving test harness that evaluates any agl-agent against archived training entities, scores recommendations against operator-validated ground truth, and uses a coach LLM to backpropagate revised rationale across generations. |

## Operator UIs

Browser-based heads-up displays for human operators to monitor, triage, and orchestrate the agents above.

| UI | Purpose |
| --- | --- |
| [email-trainer](./email-trainer/) | Triage HUD for the personal-email pipeline. Displays live entity cards, exposes quick-action buttons and hotkeys for gated human inputs (instruction, rationale, approval), and streams real-time state changes over WebSocket as the agent processes emails in bulk. |

## Repository Layout

- [personal-email/](./personal-email/) contains the current agent implementation, its microagents, and its local data.
- [lib/](./lib/) contains shared runtime helpers such as tracing, shell execution, Gmail integration, memo integration, and voice playback.

## Dependencies

- 👼 [agl-ai](https://github.com/mikesmullin/agl#readme) agent framework
- ✉️ [google-mail](https://github.com/mikesmullin/google-mail/) for Gmail integration
- 🧠 [memo](https://github.com/mikesmullin/memo/) for long-term memory
- 🤖 [MICROAGENT.md](https://github.com/mikesmullin/agl/blob/main/docs/MICROAGENT.md) maximizing effectiveness
- 🗣️ [voice](https://github.com/mikesmullin/voice) for TTS playback