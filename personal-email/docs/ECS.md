i like to use an ECS system approach to the main agent loop (agent.coffee).

What i try to do is identify sections of the loop, which can be encapsulated + extracted into a function (these fn are named `{{module}}System()`), which are defined in `systems/{{module}}.coffee`.

So instead of several code statements/lines in-a-row that represent the logic needed to utilize a microagent at that point in the loop, we just have a function call.

The spirit of a system in an entity component system architecture: It's a function that iterates/loops over a group of entities. (in the case of personal-email agent, 1 entity = 1 email.) 
Likewise, a component is a group of fields/properties that belong to an entity; so 1 entity hasMany components. and a system function chooses which entities it will operate on based on which components exist on each entity it visits. (this is like a query; select all entities with components (X,Y,Z)). So each system function determines what arbitrary grouping of entities it will iterate over, but it's the presence of components on an entity that really define the entity--because an entity with zero components is just a unique identifier (UUID or Generational Index), and can hold no information/data outside of its components.

so again applying this to our personal-email agent, an email entity might have an `.id` set equal to the hash given by `google-email` cli. that entity might initially begin its life just two components called `envelope` which contains fields {to, from, sent, subj}, and another component called `content` which contains {body}. later, as it is refined by various systems, it may gain additional components, such as `summarized` with fields like {summary, keywords}, and another commponent `recommended` with fields {recommendation, confidence}, and so on. Note how these components and fields relate/couple strongly to the outputs of the `microagents/*` (almost 1:1 ratio of component:microagent), where the output of one microagent is stored on an entity's component, and then is passed as input to the next microagent in the main engine loop sequence).

where you can help:
help me gather the inputs + outputs of each microagent into components. help me define the entity structure when emails are first pulled. also help me persist these entities to disk (as the primary authoritative source of data; rather than in-memory. make a model that can govern read/write operations to entity data, and it reads/writes to disk every time). help me rewrite the agent.coffee main engine loop so that it is now using system functions, entities, and components.

there should be nothing left in the main agent engine loop that isn't a call to a system function; this is the standard (everything is a system that would happen from that loop)



