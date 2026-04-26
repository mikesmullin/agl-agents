# personal-email

`personal-email` is an interactive email triage agent for Gmail. It pulls unread messages into a local cache, loads one email at a time, summarizes the message, recalls relevant journal and reading format context, recommends an action, and then lets the operator either accept that recommendation or issue a custom instruction.

## Run From The Command Line

From the repository root:

```bash
bun personal-email/agent.mjs
```

The agent starts an interactive unread-email triage loop. It will pull unread mail, present one email at a time, speak a short summary, show a recommended action, and then wait for operator input.

## What It Does

- Pulls unread Gmail messages through the `google-email` CLI.
- Loads cached unread email IDs in pages and processes them in a loop.
- Builds a decision input from the email envelope and cleaned body content.
- Recalls related journal context and presentation preferences from memo databases.
- Uses microagents to summarize the email, recommend an action, answer follow-up questions, and build persisted journal entries.
- Executes the chosen Gmail mutation and applies queued changes during graceful shutdown.

## Operation

Ways to answer interactive prompt:

- `proceed` or `p` to apply the recommended action.
- `quit` to run graceful shutdown, apply queued Gmail mutations, and exit.
- A custom instruction to override the recommendation.
- A memo-related command to operate on the journal or presentation memo databases.
- A natural-language question about the current email.

## Key Files

- [agent.mjs](./agent.mjs): top-level interactive loop.
- [microagents/](./microagents/): single-purpose model-driven decision units.
- [db/](./db/): local memo databases used for journal history and presentation preferences.

## Microagents

For a more detailed definition of what a microagent is and how to structure one, see [the upstream MICROAGENT guide](https://github.com/mikesmullin/agl/blob/main/docs/MICROAGENT.md).

- [01-recommend-action.mjs](./microagents/01-recommend-action.mjs): recommends what to do with the current email.
- [02-contains-question.mjs](./microagents/02-contains-question.mjs): decides whether the operator input is a question.
- [04-answer-question-from-email.mjs](./microagents/04-answer-question-from-email.mjs): answers operator questions from the email content.
- [05-summarize-email.mjs](./microagents/05-summarize-email.mjs): generates the headline and summary text shown to the operator.
- [06-presentation-rule-relevance.mjs](./microagents/06-presentation-rule-relevance.mjs): checks whether saved formatting preferences apply to the current email.
- [07-execute-memo-instruction.mjs](./microagents/07-execute-memo-instruction.mjs): handles memo database instructions.
- [08-execute-instruction.mjs](./microagents/08-execute-instruction.mjs): executes Gmail actions derived from the final instruction.
- [09-build-journal-entry.mjs](./microagents/09-build-journal-entry.mjs): turns the outcome into a journal record.
- [10-build-presentation-entry.mjs](./microagents/10-build-presentation-entry.mjs): extracts reusable presentation preferences.

## Configuration & Data

- Root [config.yaml.example](../config.yaml.example) shows the optional folder-description config shape.
- Local [db/](./db/) stores journal and presentation memo state.
- Shared helpers live in [../lib/](../lib/).

## Example

Example session with `lm-studio:google/gemma-4-e4b` (9B param) on RTX 5070Ti (16MB VRAM) Context: 131072

```
[user@myarch agents]$ bun personal-email/agent.mjs 
🗃️ Loaded Gmail folder cache. (795 ms)
📥 Pulled latest emails. (2213 ms)
📋 Loaded unread inbox page. (444 ms)
📨 Loaded email d93ff1. (411 ms)
🧽 Pre-filtered email markup. (2 ms)
🔎 Searched memo journal context. (121 ms)
🎛️ Searched presentation memo context. (109 ms)
🎯 Extracted presentation preferences. (0 ms)
🧠 Summarized email. (869 ms)
🗺️ Generated recommendation. (1133 ms)

========== NEXT EMAIL ==========
From: Safety Posts <reply@ss.email.nextdoor.com>
Subj: Man arrested after allegedly attempting to get into
Date: 2026-04-11T16:17:09.000Z
🗣️ Summary: NSS Alert: Keep Car Doors Locked

Nextdoor post advises keeping car doors locked when parked anywhere, including at home. Post is about an arrest in the area.

Recommended action:
(Journal 3) mark as read + move to Newsletters. The email is a local neighborhood alert (Nextdoor) about property safety. Treat routine community posts like other Nextdoor digests and archive them after marking as read..
===============================

🤖 What would you like to do?
(proceed, quit)> Why are people recommending this? 
❓ Checked if instruction is a question. (323 ms)
🤖 Analyzed email for your question. (659 ms)
🗣️ Answer: The provided content is an alert from Safety Posts recommending that people keep their car doors locked when they are in the car or anywhere they park, including at home. This recommendation follows an incident involving a man allegedly attempting to break into a vehicle.
🤖 What would you like to do?
(proceed, quit)> delete
❓ Checked if instruction is a question. (181 ms)
⚙️ Executed your instruction. (865 ms)
✅ The email has been deleted as requested.
📝 Built journal entry. (630 ms)
💾 Saved journal to memo. (157 ms)
🧩 Built presentation entry. (479 ms)
📚 Saved presentation memo. (113 ms)
📨 Loaded email e500b9. (414 ms)
🧽 Pre-filtered email markup. (1 ms)
🔎 Searched memo journal context. (123 ms)
🎛️ Searched presentation memo context. (145 ms)
🎯 Extracted presentation preferences. (0 ms)
🧠 Summarized email. (871 ms)
🗺️ Generated recommendation. (1160 ms)

========== NEXT EMAIL ==========
From: Safety Posts <reply@ss.email.nextdoor.com>
Subj: On April 9 at approximately midnight, this female subject attempted
Date: 2026-04-11T14:18:07.000Z
🗣️ Summary: Attempted Bike Theft Report (4/9)

Report of attempted bike theft by female subject near the city center on April 9 (midnight). See attached Nextdoor post for details.

Recommended action:
(Guess) mark as read; move to archive. This is an informal neighborhood safety post (Nextdoor) about a suspicious incident. It's not relevant for specific folders and can be archived after review..
===============================

🤖 What would you like to do?
(proceed, quit)> delete
❓ Checked if instruction is a question. (323 ms)
⚙️ Executed your instruction. (793 ms)
✅ I have deleted the email as requested.
📝 Built journal entry. (1116 ms)
💾 Saved journal to memo. (166 ms)
🧩 Built presentation entry. (538 ms)
📚 Saved presentation memo. (110 ms)
📨 Loaded email 1b1b4c. (405 ms)
🧽 Pre-filtered email markup. (1 ms)
🔎 Searched memo journal context. (127 ms)
🎛️ Searched presentation memo context. (111 ms)
🎯 Extracted presentation preferences. (0 ms)
🧠 Summarized email. (835 ms)
🗺️ Generated recommendation. (1278 ms)

========== NEXT EMAIL ==========
From: Only Deals <jake@evergreenavenue.ccsend.com>
Subj: Reminder: Clean Flip  - Assignable Contract
Date: 2026-04-11T11:01:45.000Z
🗣️ Summary: Clean Flip Deal - Assignable Contract Reminder

Reminder for Clean Flip deal. PP $439K, ARV $630K+. Assignable contract info and contact provided (Jake). Due diligence needed.

Recommended action:
(Guess) mark as read + move to Real Estate Investment. This is a commercial email promoting real estate deals (flip/assignment contract). The destination 'Real Estate Investment' was created for this purpose..
===============================

🤖 What would you like to do?
(proceed, quit)> p
Proceed mode: applying recommended action without journal update.
⚙️ Executed recommended action. (2050 ms)
✅ Marked the email as read and moved it to the 'Real Estate Investment' folder per your instructions.

📥 Refreshed emails at page boundary. (2184 ms)
📋 Reloaded unread inbox page. (436 ms)
No unread emails remain after refresh. Running graceful shutdown...
🧾 Planning queued mutations...
📋 3 email(s) with pending mutations:

  0426ac        Man arrested after allegedly attempting to get ...
    → delete (trash)

  e500b9        On April 9 at approximately midnight, this fema...
    → delete (trash)

  1b1b4c        Reminder: Clean Flip  - Assignable Contract
    → mark as read
    → move to "Real Estate Investment"

Plan: 4 action(s) on 3 email(s)

Run 'google-email apply' to execute these changes on Gmail.
🧾 Planned queued mutations. (410 ms)
🚀 Applying queued mutations...
Applying 4 action(s) to 3 email(s)...

  ✓ 0426ac      delete
  ✓ e500b9      delete
  ✓ 1b1b4c      mark read
  ✓ 1b1b4c      move → Real Estate Investment

✓ Successfully applied 4 action(s).
🚀 Applied queued mutations. (3282 ms)
🧹 Cleaning local google-email cache...
✓ Deleted 3 cached email file(s) from storage/.
🧹 Cleaned local google-email cache. (437 ms)
[user@myarch agents]$
```