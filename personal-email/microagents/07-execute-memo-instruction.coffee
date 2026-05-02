import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { YAML } from 'bun'
import { _G } from '../../lib/globals.coffee'
import { mkdir, writeFile, rm } from 'fs/promises'
import { resolve } from 'path'

_G.executeMemoInstructionMicroagent = (instruction) ->
  _G.traceStep '🧾', 'Applying memo bookkeeping', ->
    microagent = await Agent.factory
      system_prompt: 'You perform memo database operations according to user-instruction.'
      output_tool:
        parameters:
          success:
            type: 'boolean'
            description: 'Successfully performed user-instruction?'
          summary:
            type: 'string'
            description: 'Description of changes made.'
        required: ['success', 'summary']

    microagent.Tool 'create_record', 'Create a new memo record.',
      db:
        type: 'string'
        description: 'Target database. Must be one of: journal, presentation.'
      body:
        type: 'string'
        description: 'Content of the new memo record.'
    , ['db', 'body'], (ctx, { db, body }) ->
      target = _G.resolveMemoDbTargetLib db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB
      return target.message if not target.ok

      await mkdir _G.DB_DIR, { recursive: true }
      tmpFile = resolve _G.DB_DIR, "memo-create-#{Date.now()}.yaml"
      yamlDoc = YAML.stringify { metadata: { ts: new Date().toISOString() }, body }
      await writeFile tmpFile, yamlDoc, 'utf8'
      save = await _G.spawn 'memo', ['save', '-f', target.path, tmpFile]
      await rm tmpFile, { force: true }
      if save.code is 0 then 'Created memo record.' else "Failed to create memo: #{save.stderr} #{save.stdout}"

    microagent.Tool 'search_records', 'Perform a semantic search against memo database.',
      db:
        type: 'string'
        description: 'Target database. Must be one of: journal, presentation.'
      query:
        type: 'string'
        description: 'Search query used for memo recall.'
      k:
        type: 'integer'
        description: 'Optional top-k results (default 5).'
      filter:
        type: 'string'
        description: 'Optional memo metadata filter expression.'
    , ['db', 'query'], (ctx, { db, query, k, filter }) ->
      target = _G.resolveMemoDbTargetLib db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB
      return target.message if not target.ok

      args = ['recall', '-f', target.path, '-k', String(if Number.isInteger(k) and k > 0 then k else 5)]
      if String(filter ? '').trim()
        args.push '--filter', String(filter).trim()
      args.push String(query)
      r = await _G.spawn 'memo', args
      if r.code is 0 then r.stdout or 'No memo results.' else "Failed to read memo database: #{r.stderr} #{r.stdout}"

    microagent.Tool 'update_record', 'Overwrite a memo record by id with new body text in the selected database. IMPORTANT: It is recommended to read the latest state of the memo record first, before attempting to edit it.',
      db:
        type: 'string'
        description: 'Target database. Must be one of: journal, presentation.'
      memo_id:
        type: 'integer'
        description: 'Memo id to overwrite, e.g. 21.'
      body:
        type: 'string'
        description: 'Full replacement body text for the memo record.'
    , ['db', 'memo_id', 'body'], (ctx, { db, memo_id, body }) ->
      target = _G.resolveMemoDbTargetLib db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB
      return target.message if not target.ok

      res = await _G.overwriteMemoByIdLib _G.spawn, _G.DB_DIR, target.path, memo_id, String(body ? '')
      if res.ok then "Updated memo #{memo_id}." else "Failed to update memo #{memo_id}: #{res.message}"

    microagent.Tool 'delete_record', 'Mark a memo as deleted by id in the selected database (writes a deletion tombstone).',
      db:
        type: 'string'
        description: 'Target database. Must be one of: journal, presentation.'
      memo_id:
        type: 'integer'
        description: 'Memo id to delete, e.g. 21.'
      reason:
        type: 'string'
        description: 'Optional reason for deletion.'
    , ['db', 'memo_id'], (ctx, { db, memo_id, reason }) ->
      target = _G.resolveMemoDbTargetLib db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB
      return target.message if not target.ok

      res = await _G.deleteMemoByIdLib _G.spawn, _G.DB_DIR, target.path, memo_id, String(reason ? '')
      if res.ok then "Deleted memo #{memo_id}." else "Failed to delete memo #{memo_id}: #{res.message}"

    microagent.Tool 'reindex_database', 'Reindex one memo database so manual YAML edits are reflected in search/recall.',
      db:
        type: 'string'
        description: 'Target database. Must be one of: journal, presentation.'
    , ['db'], (ctx, { db }) ->
      target = _G.resolveMemoDbTargetLib db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB
      return target.message if not target.ok

      r = await _G.reindexMemoDbLib _G.spawn, target.path
      if r.code isnt 0
        return "Failed to reindex #{target.db}: #{r.stderr} #{r.stdout}"
      await _G.gitJournalCommitLib _G.spawn, _G.DB_DIR, "journal: reindex #{target.db}"
      "Reindexed #{target.db} memo database."

    prompt = """
      <user-instruction>
      #{_G.xmlEscape instruction}
      </user-instruction>
      """

    result = await microagent.run { prompt }
    _G.log 'microagent.result',
      name: 'executeMemoInstructionMicroagent'
      input: { instruction }
      output: result
    , 'microagent'
    result
