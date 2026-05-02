# Prompt History

(This is a history of prompts I used to build this repo/project. They have already been applied, and are kept here for historical reference.)

---

context:
- README.md: read this to understand our project
- personal-email/README.md: read this to understand our first agent that we are working on
- personal-email/docs/ECS.md: read this to understand the ECS concept we've just implemented

changes to make:
- personal-email/agent.coffee:107~135: in my ECS architecture, its an anti-pattern to pass `entity` or `store` this way. each system fn is responsible to retrieve the list of entities it wants to operate on, and loop over them (always assuming there will be a list of entities available in the World)
- personal-email/systems/entity-store.coffee: move this to personal-email/models/entity.coffee

instead of passing entity and store to each system function:
- at the top of each system function should be some statements that are capabple of querying the list of entities in our World
  - we're missing the concept of a World (it starts empty initially)
    - upon email pull, the World is populated with one entity per email
      - where the entity initially only has an .id set equal to the email id (a hash)
- the World should have a `models/world.coffee` which is a Model (in the context of Model, View, Controller architecture--its the M)
  - it should provide a type of query fn like `World.Entity__find(filterByFn)` which can be used to select the entities needed by the system fn (even if there is only one email in the world, which might be the case for now... but will change in the future; we are migrating from a serial(one email at a time, processed beginning to end) agent main loop to a highly concurrent one (batch of ~10 emails, processed in stages))

also, for the EntityStore class
- i would prefer instead of being OOO in the sense of using class instances, let's do it like this (more like a Golang class)
  - one static class (never isntantiated)
  - pass the entity instance as required first parameter to each of its "instance" methods
- this way we don't need to keep track (and pass) one `store` instance per-entity. we just need to pass the current `entity` in like `EntityStore.save(entity)`





- personal-email/systems/execute.coffee:14~17: we have a pattern where commonly we call this at the end of a System fn; but we can save some lines (and DRY up this code) by moving this into the `Entity.patch()` fn; any time a patch is applied, we call _G.log().



good. more improvements: (fixing our logs+traces to be (batchable, and persisted to disk))
- (personal-email/systems/reload.coffee:25~26, personal-email/systems/reload.coffee:24~25,20~21): we have this pattern throughout our System functions where we call {(traceStart, traceEnd), console.log} (similarly in other places we may call _G.log) but (in the new paradigm shift (of moving from (serial (one email at a time) to concurrent (batch of ~10 emails, processed in stages)))) this approach no longer works (it would emit to stdout in a highly interleaved way that would be difficult for human readers to follow any particular entity/email's journey from beginning to end (first stage to last)). therefore, the new goal is to write/append logs+traces to entity yaml via the entity model (move these fns to become static methods on that class; store the data under a `log:` key (string array, one item per entry) and similarly a `traces:` key (array of trace objects, stored upon `traceEnd`)). also there were some other _G.trace*() fns which we don't really use anymore (or shouldn't), which you should find/remove (and replace any calls to those fns with one of these instead).


...

- personal-email/systems/reload.coffee:1~3: instead of requiring so many files to include an import statement for (`World` and `Entity`), have these files define themselves via `_G`, so that `agent.coffee` imports them once, and then all other files can inherit access simply by importing `_G` and referencing `_G.World` or `_G.Entity`. (this makes the code more DRY and convenient to read/write).


...

- instead of 
```
export class Entity
...
_G.Entity = Entity
```

use this pattern:
```
_G.Entity = class Entity
...

use this pattern for `World` also

...

good. more improvements: (moving from readline/interactive -> unattended/headless operation)
paradigm shift:
- before (syncrhonous, foreground process): The human operator was I was waiting, watching every step of the a serial email processing loop. 
- after (asynchronous, background systemctl service process): the human operator will not always be present.
in order to make this possible, we move the gate function (from readline prompts) -> (to boolean values in each entity's state yaml). then we expect that the user will edit these files directly on disk; And this will be their means of both answering questions and providing specific instructions necessary to progress the entity to the next stage. .

just one example:
- (personal-email/agent.coffee:63~64,139~140): we will no longer be using readline to communicate with the human operator.
- personal-email/agent.coffee:121~133: the command value will be set via the entity state (read-in via entity model from entity yaml on disk). (depending on the rate/frequency that our main agent loop runs, There will be many loop iterations where entities do not have value set (The human operator has not Reviewed the entity file yet)... Therefore.. a null value here should be Valid, but should also gate the email/entity from progressing to the next stage, without blocking the loop for all other entities to be successfully processed as as appropriate)

applying to all personal-email/systems/*
This is just one example of how user interaction will fundamentally change throughout each System. So we'll need to continue identifying areas where systems require/gate/await on user input, and translate that to an appropriate area within the entity state file (likely, a new component (a special subtype, called an Input Component) with one field per user input required), and update the logic to gate on that value (in an asynchronous way (by this i do not mean the `async`/`await` keywords, but rather a simple pattern of (pseudocode: if empty then no-op, else process stage for this entity) which is the best kind of async (it's one that would work across multiple runs of the process; since our state is fully serialized to disk, our process could crash--and it would still recover right where it left off for every entity, without skipping a beat); without blocking the main loop).



...

because we are now in the (batch processing, and unattended headless operation) mindset,
- personal-email/systems/display.coffee:16~17: emitting voice (TTS audio) no longer makes sense. it would be too chaotic, with many voices speaking (interleaved). remove calls to `_G.speakText()` (but you can keep the fn definition around, as we may use it again later (i'm planning a new kind of user interface (one that is focused around batch/bulk processing), but i haven't described this yet; it will be later))
- lib/perception.coffee: similarly, any usage of the perception listening (STT) feature should have references removed (but, again, preserve this file as we may use its functions at a later time)


now let's update our documentation a bit:

- tmp/MEMORY_PRD.md:296~312: similar to this ordering (but now as a table instead of a bullet list), create an updated list which describes order-of-operations (one row per system) that systems execute in the agent main loop, and for each stage/row: one column "inputs" which lists the path in entity yaml (comma-separated list) where component fields are going to be used as input to that stage (for any inputs that must be provided by the user (gated; stage can't begin without them) use an emoji to indicate those, so the user reading this doc will know how to provide them), and similarlry another for "outputs".. write this table to the personal-email README.

...

- personal-email/agent.coffee:90~91,72~73: instead of this its easier to set _G.quit=true when/where we want to exit the loop and have the while loop defined as `while !_G.quit`

...

the concept of `quit` (and its graceful shutdown operations (plan, apply, clean)) no longer apply (in the headless service loop strategy).
- personal-email/README.md:51~52: instead we should update code (and then this line in the doc) so that the (plan) is a new (appended to end of loop) stage of the entity in the agent loop. likewise (apply) will also be a new stage, but it will be gated by user input (the user will be reviewing the entity yaml to see if they agree with the plan; they will indicate their approval via a boolean to a new input component). however the final operation (clean) will never be applied. this will be controlled externally by the human operator (for now; we can keep the fns around, because i may use them when i recreate the UI/UX later). 

...

when i `bun run personal-email/agent.coffee`,
it successfully pulls + processes 10 emails
up to the point of the first user input gate (which is good).
but then it stops making progress.
i assume the loop is still iterating.

but what i am expecting is that it would move to 
(pull the next page of emails,
process that batch,
and then repeat--
until there are no more pages to pull;
because it has reached the end of the email pull range (default: "14 days ago"....
but despite this, the loop would keep iterating,
in case a user edits one of the yaml files,
which would allow that email to progress to next gate/stage)
).


how can we improve the way pagination is happening in our agent.coffee main loop?

...

ok i decided it would be easiser if i patched `google-email` cli; we now have two new features: (read the output of `skills google-email` (its SKILLS.md file) to find a detailed description of each)

- `google-email clear [--erase] <id>`: (new subcommand. read its `--help`). this will reset/undo any changes in the offline cache that are queued/pending `google-email apply`. (so we can have BEGIN...COMMIT/ROLLBACK transaction-style flow; will be useful later)

- `transitions` array now catalog when changes are detected on the remote side of the email inbox (ie. new, deleted, moved, etc.). on each pass of our agent loop, we should be looking for these entries to see if anything has changed since last time (we should keep track of a new component field lastCheckedAt). then we'll know: if there are new emails then add them to World. if there are deleted emails then remove the entity from World and disk. if there are moved emails then update the state for the entity on disk. this can all happen as the first system that runs in the agent loop (responsible for pull). this way the `pull` System is simple: we just keep pulling (all emails, not limited to 10) for the date range (ie. `14 days ago`) and then checking transitions[] to see if there have been any differences since lastCheckedAt, and if there are, then we act. else if there aren't then we no-op and return control to the  proceed to next system in the main agent loop.

...

- made a change `oprator_input.instruction` (null -> "proceed") on `entity.id=c140ac`, but the next iteration of the main agent loop did not detect the change--i am expecting every read happens via Entity model, and every read operation loads from disk (not relying on in-memory cached version). is this true? (seems not.) if not, fix.

