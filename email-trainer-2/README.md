# email-trainer-2

A Gmail/Outlook-style triage UI for the `personal-email` agent — a redesign of
[`email-trainer`](../email-trainer/). Same backend data flow (reads
`personal-email/db/entities/`, writes operator input via REST PATCH, streams
state over WebSocket), but a classic list-based inbox interface instead of the
card feed.

## Run

```bash
# from this folder
bun start          # → http://localhost:4001
```

Runs alongside the original `email-trainer` (port 4000) without conflict.

## What's different from v1

- **Classic inbox layout** — a fixed top toolbar over a vertical list of email
  rows, instead of a feed of vertical cards. The toolbar never moves, so rapid
  clicking of **Proceed** (or any bulk action) stays on target.
- **Multi-select bulk actions** — each row has a checkbox; toolbar actions apply
  to all selected emails at once (or the open/focused email).
- **Progressive-disclosure action bar** — clicking a toolbar action expands a
  second, collapsible toolbar with the relevant fields (folder for *Move*,
  instruction + rationale for most ops, capture/display for *Notice*) and a
  **Submit** button.
- **Detail page** — clicking a row opens a full-width detail view (sidebar,
  search and toolbar stay put) showing the stage-contextual summary plus the
  **original email rendered in a sandboxed `<iframe>`** (`sandbox=""` → scripts
  and same-origin disabled, so no XSS).
- **No confetti, no auto-refilling card slots.**
- **Star / important** toggles per row (stored client-side in `localStorage`;
  they do not modify entity files).

## Preserved from v1

- Dark theme; Alpine.js + Tailwind + Phosphor icons; Bun + CoffeeScript backend.
- Stage sidebar with eye toggle, rainbow stage colors, and per-stage counts.
- Operator History, Trial History, and Trial Run pages (unchanged).
- Stage-specific hotkeys, WebSocket live updates, and CSS transitions.

## Configuration

Configuration is hard-coded (no `config.yaml`):

| Setting | Value |
|---------|-------|
| Port | `4001` |
| Entities dir | `../personal-email/db/entities` |
| Poll interval | `3000 ms` |

Move-destination folders are read from `personal-email/config.yaml`
(`google_email.labels`), falling back to a built-in preset.

## REST API

Same as v1 plus one addition:

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/entities` | Live entities (`origin.raw` stripped) |
| `GET` | `/api/entities/:id/raw` | Original email body (HTML + text) for the detail view |
| `GET` | `/api/config` | Port, poll interval, move destinations |
| `PATCH` | `/api/entities/:id` | Deep-merge patch into entity YAML |
| `GET`/`DELETE` | `/api/archive`, `/api/archive/:id` | Operator history |
| `GET`/`DELETE`/`POST` | `/api/trials…` | Trial history + promote |
| `GET`/`POST` | `/api/trial-run/…` | Trial run control |

### Hotkeys

| Key | Action |
|-----|--------|
| `j` / `k` | Focus next / previous row |
| `x` | Toggle selection of focused row |
| `Enter` | Open focused row |
| `p` | Proceed (apply recommendation / approve) on focused row |
| `s` / `d` / `a` | Skip / delete / archive (awaiting-input); `a` also approves |
| `r` | Re-process a skipped email |
| `/` | Focus search |
| `Esc` | Close action bar → close detail → clear search → clear filter |
