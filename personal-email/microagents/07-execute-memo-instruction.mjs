import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { YAML } from 'bun';
import { _G } from '../../lib/globals.mjs';
import { mkdir, writeFile, rm } from 'fs/promises';
import { resolve } from 'path';

_G.executeMemoInstructionMicroagent = async (instruction) => {
  return _G.traceStep('🧾', 'Applying memo bookkeeping', async () => {
    const microagent = await Agent.factory({
      system_prompt: `You perform memo database operations according to user-instruction.`,
      output_tool: {
        parameters: {
          success: { type: 'boolean', description: 'Successfully performed user-instruction?' },
          summary: { type: 'string', description: 'Description of changes made.' },
        },
        required: ['success', 'summary'],
      },
    });

    microagent.Tool('create_record', 'Create a new memo record.', {
      db: { type: 'string', description: 'Target database. Must be one of: journal, presentation.' },
      body: { type: 'string', description: 'Content of the new memo record.' },
    }, ['db', 'body'], async (ctx, { db, body }) => {
      const target = _G.resolveMemoDbTargetLib(db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB);
      if (!target.ok) return target.message;

      await mkdir(_G.DB_DIR, { recursive: true });
      const tmpFile = resolve(_G.DB_DIR, `memo-create-${Date.now()}.yaml`);
      const yamlDoc = YAML.stringify({ metadata: { ts: new Date().toISOString() }, body });
      await writeFile(tmpFile, yamlDoc, 'utf8');
      const save = await _G.spawn('memo', ['save', '-f', target.path, tmpFile]);
      await rm(tmpFile, { force: true });
      return save.code === 0 ? 'Created memo record.' : `Failed to create memo: ${save.stderr} ${save.stdout}`;
    });

    microagent.Tool('search_records', 'Perform a semantic search against memo database.', {
      db: { type: 'string', description: 'Target database. Must be one of: journal, presentation.' },
      query: { type: 'string', description: 'Search query used for memo recall.' },
      k: { type: 'integer', description: 'Optional top-k results (default 5).' },
      filter: { type: 'string', description: 'Optional memo metadata filter expression.' },
    }, ['db', 'query'], async (ctx, { db, query, k, filter }) => {
      const target = _G.resolveMemoDbTargetLib(db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB);
      if (!target.ok) return target.message;

      const args = ['recall', '-f', target.path, '-k', String(Number.isInteger(k) && k > 0 ? k : 5)];
      if (String(filter || '').trim()) {
        args.push('--filter', String(filter).trim());
      }
      args.push(String(query));
      const r = await _G.spawn('memo', args);
      return r.code === 0 ? r.stdout || 'No memo results.' : `Failed to read memo database: ${r.stderr} ${r.stdout}`;
    });

    microagent.Tool('update_record', 'Overwrite a memo record by id with new body text in the selected database. IMPORTANT: It is recommended to read the latest state of the memo record first, before attempting to edit it.', {
      db: { type: 'string', description: 'Target database. Must be one of: journal, presentation.' },
      memo_id: { type: 'integer', description: 'Memo id to overwrite, e.g. 21.' },
      body: { type: 'string', description: 'Full replacement body text for the memo record.' },
    }, ['db', 'memo_id', 'body'], async (ctx, { db, memo_id, body }) => {
      const target = _G.resolveMemoDbTargetLib(db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB);
      if (!target.ok) return target.message;

      const res = await _G.overwriteMemoByIdLib(_G.spawn, _G.DB_DIR, target.path, memo_id, String(body || ''));
      return res.ok ? `Updated memo ${memo_id}.` : `Failed to update memo ${memo_id}: ${res.message}`;
    });

    microagent.Tool('delete_record', 'Mark a memo as deleted by id in the selected database (writes a deletion tombstone).', {
      db: { type: 'string', description: 'Target database. Must be one of: journal, presentation.' },
      memo_id: { type: 'integer', description: 'Memo id to delete, e.g. 21.' },
      reason: { type: 'string', description: 'Optional reason for deletion.' },
    }, ['db', 'memo_id'], async (ctx, { db, memo_id, reason }) => {
      const target = _G.resolveMemoDbTargetLib(db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB);
      if (!target.ok) return target.message;

      const res = await _G.deleteMemoByIdLib(_G.spawn, _G.DB_DIR, target.path, memo_id, String(reason || ''));
      return res.ok ? `Deleted memo ${memo_id}.` : `Failed to delete memo ${memo_id}: ${res.message}`;
    });

    microagent.Tool('reindex_database', 'Reindex one memo database so manual YAML edits are reflected in search/recall.', {
      db: { type: 'string', description: 'Target database. Must be one of: journal, presentation.' },
    }, ['db'], async (ctx, { db }) => {
      const target = _G.resolveMemoDbTargetLib(db, _G.MEMO_DB, _G.PRESENTATION_MEMO_DB);
      if (!target.ok) return target.message;

      const r = await _G.reindexMemoDbLib(_G.spawn, target.path);
      if (r.code !== 0) {
        return `Failed to reindex ${target.db}: ${r.stderr} ${r.stdout}`;
      }
      await _G.gitJournalCommitLib(_G.spawn, _G.DB_DIR, `journal: reindex ${target.db}`);
      return `Reindexed ${target.db} memo database.`;
    });

    const prompt = `
<user-instruction>
${_G.xmlEscape(instruction)}
</user-instruction>
`;

    const result = await microagent.run({ prompt });
    _G.log('microagent.result', {
      name: 'executeMemoInstructionMicroagent',
      input: { instruction },
      output: result,
    }, 'microagent');
    return result;
  });
}
