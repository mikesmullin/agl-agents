# email-trainer

`email-trainer` is a browser-based triage UI for the `personal-email` agent. It makes it fast and easy for a human operator to review, gate, and approve email entities as they flow through the personal-email pipeline — enabling bulk training-data collection at scale (goal: 100+ recorded emails).

## What It Does

- **Reads** the `personal-email/db/entities/` directory in real-time via WebSocket push. No page refreshes needed.
- **Renders** each email entity as a card in a Twitter-style vertical feed, with a layout that adapts to the entity's current pipeline stage.
- **Gates** on the two human-required stages:
  - **Awaiting Input** (`operator_input.instruction === null`) — operator fills in instruction, rationale, and optional notice fields, or clicks a quick-action button.
  - **Awaiting Approval** (`apply.approved === null`) — operator reviews the mutation plan and clicks Approve or Reject.
- **Writes** input directly back into the entity YAML file via a REST PATCH, so the personal-email agent picks it up on its next loop iteration (every ~10 s).
- **Tracks** pipeline progress with a rainbow-colored sidebar that shows per-stage entity counts. Clicking a stage filters the feed.

## Install

No additional dependencies required. Uses the same Bun + CoffeeScript setup as the rest of the repository.

```bash
# From the repository root:
bun install       # install deps (if not already done)
bun start         # start the email-trainer server
```

The server prints its URL to stdout on startup:

```
  ✉️  email-trainer
  🌐  http://localhost:4000
  📁  /workspace/agl-agents/personal-email/db/entities
```

Click the URL (most terminals support clicking) to open the UI in your browser.

## Configuration

Copy `email-trainer/config.yaml.example` to `email-trainer/config.yaml` (done automatically on first run if the file is absent) and edit as needed:

```yaml
port: 4000                              # HTTP + WebSocket port
entities_dir: ../personal-email/db/entities   # path to entity YAML files (relative to email-trainer/)
poll_interval_ms: 3000                  # how often to check for file changes (ms)
destinations:                           # available Gmail destination folders shown in Move… picker
  - Expenses
  - Statements
  - Newsletters
  # …etc
```

## Running the UI

1. Start `personal-email` agent in a separate terminal:
   ```bash
   bun personal-email/agent.coffee
   ```
2. Start `email-trainer`:
   ```bash
   bun start
   ```
3. Open `http://localhost:4000` in your browser.

As the personal-email agent processes emails, entities appear automatically in the feed. Cards awaiting human input glow amber. Cards awaiting final approval glow blue.

## Triage Workflow

### Stage: Awaiting Input ✏️

For each email the agent has finished analyzing, it writes `operator_input.instruction: null` as a gate. The UI shows:

- **Email header** — from, subject, date
- **AI summary** — headline + description
- **Recommendation chip** — what the agent wants to do (e.g. `move to Statements`) + confidence %

**Quick-action buttons** (one click = instant submit):

| Button | Instruction sent |
|--------|-----------------|
| ✅ Proceed | `proceed` — apply the agent's recommendation |
| ⏭️ Skip | `skip` — skip without executing |
| 📦 Archive | `archive` — remove from inbox without moving |
| 🗑️ Delete | `delete` — trash the email |
| 📁 Move… | `move to {folder}` — choose destination |
| 📢 Notice | reveals `notice_capture` + `notice_display` fields |

**Form fields** (for custom or notice inputs):

- **Instruction** — free-text or pre-filled recommendation; `Enter` = submit
- **Rationale** — 1–2 sentences explaining why; seeds the `trial_rationale` for trial-runner
- **notice_capture** / **notice_display** — shown only when Notice button is active

After submitting, a confetti animation plays for `proceed` actions.

### Stage: Awaiting Approval ☑️

After the agent writes a mutation plan, it gates again on `apply.approved`. The UI shows the plan text and two buttons:

- **✅ Approve & Apply** — sets `apply.approved: true`; confetti plays
- **↩️ Reject / Re-process** — sends a `refresh` instruction to clear state and re-run the pipeline

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll to next / previous card |
| `p` | Quick **proceed** on the focused card |
| `s` | Quick **skip** on the focused card |
| `a` | Quick **approve** on the focused card |
| `f` | Toggle filter to **Awaiting Input** only |
| `Esc` | Clear active filter |

### Sidebar

The left sidebar shows entity counts per pipeline stage, each with a rainbow-gradient color badge. Clicking a stage filters the main feed to show only entities in that stage. Clicking the same stage again (or pressing `Esc`) clears the filter.

Human-gated stages are marked with 👤 and float to the top of the feed by default.

## File Watching

The server polls `entities_dir` every `poll_interval_ms` milliseconds, comparing file modification timestamps and sizes to detect:

- **New entity** — a `.yaml` file that wasn't there before → card slides in
- **Modified entity** — an existing file that changed (e.g., agent advanced the pipeline) → card updates reactively
- **Deleted entity** — a file that disappeared (agent archived it) → card slides out

All changes are broadcast to all connected browser tabs via WebSocket. No page refresh needed.

## Persistent State

`email-trainer/db/state.yaml` is reserved for UI state persistence (e.g., last active filter). It is created automatically if absent.

## Architecture

| File | Purpose |
|------|---------|
| `server.coffee` | Bun HTTP + WebSocket server; file watcher; REST API (`GET /api/entities`, `PATCH /api/entities/:id`, `GET /api/config`) |
| `config.yaml` | Active configuration |
| `config.yaml.example` | Example / default configuration |
| `public/index.html` | SPA entry point; Alpine.js, Tailwind CSS, Lucide icons, canvas-confetti (all via CDN) |
| `public/js/app.js` | Alpine.js application; WebSocket client; stage detection; triage actions |
| `public/css/styles.css` | Custom CSS; animations; dark theme tweaks |
| `db/state.yaml` | Persistent UI state |

### REST API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/entities` | All live entities (origin.raw stripped) |
| `GET` | `/api/config` | Server config (destinations list, etc.) |
| `PATCH` | `/api/entities/:id` | Deep-merge patch into entity YAML |

### WebSocket (`/ws`)

| Direction | Message | Description |
|-----------|---------|-------------|
| Server → Client | `{ type: 'init', entities: [...] }` | Full entity list on connect |
| Server → Client | `{ type: 'entity:new', entity }` | New YAML file detected |
| Server → Client | `{ type: 'entity:modified', entity }` | File changed |
| Server → Client | `{ type: 'entity:deleted', id }` | File removed |

## Relationship to Other Agents

```
personal-email/agent.coffee   ─── writes entity YAML files ──→  personal-email/db/entities/
                                                                          ↑↓ disk
email-trainer/server.coffee   ─── polls + serves ──→  browser UI  ←─── operator input
                                                                          ↓ PATCH → disk
trial-runner/agent.coffee     ─── reads archived entities ──→  training dataset
```

The email-trainer does not call Gmail, modify memo databases, or invoke LLMs. It is purely a human-input accelerator for the entities already in-flight through the personal-email pipeline.
