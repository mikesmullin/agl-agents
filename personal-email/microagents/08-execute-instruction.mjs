import Agent from '../../node_modules/agl-ai/src/agent.mjs';
import { _G } from '../../lib/globals.mjs';

export const executeInstructionMicroagent = _G.executeInstructionMicroagent = async (emailId, instruction) => {
  const microagent = await Agent.factory({
    system_prompt: `You perform email operations on behalf of the user.`,
    output_tool: {
      description: 'Outcome of executing the user instruction for this email.',
      parameters: {
        success: { type: 'boolean', description: `You were successful?` },
        summary: { type: 'string', description: 'Short summary of the action(s) you took.' },
      },
      required: ['success', 'summary'],
    },
  });

  microagent.Tool('mark_as_read', 'Mark the email as read (offline metadata).', {}, [], async () => {
    const r = await _G.emailRead(emailId);
    return r.code === 0 ? 'Marked as read.' : `Failed: ${r.stderr} ${r.stdout}`;
  });

  microagent.Tool('mark_as_unread', 'Mark the email as unread (offline metadata).', {}, [], async () => {
    const r = await _G.emailUnread(emailId);
    return r.code === 0 ? 'Marked as unread.' : `Failed: ${r.stderr} ${r.stdout}`;
  });

  microagent.Tool('delete', 'Queue the email for deletion (moved to Deleted Items when applied).', {}, [], async () => {
    const del = await _G.emailDelete(emailId);
    if (del.code !== 0) {
      return `Failed to queue deletion: ${del.stderr} ${del.stdout}`;
    }
    await _G.emailRead(emailId); // mark-as-read is implicit
    return 'Queued for deletion.';
  });

  microagent.Tool('move', 'Queue the email to be moved to a folder (including Archive).', {
    folder: { type: 'string', description: 'Destination folder name. Must match existing (case-sensitive).' },
  }, ['folder'], async (ctx, { folder }) => {
    const requestedFolder = _G.mustBeTrimmedStringOr(folder, 'Unknown');

    const move = await _G.emailMove(emailId, requestedFolder);
    if (move.code !== 0) {
      return [move.stderr, move.stdout].filter(Boolean).join(' ') || 'Failed to queue move.';
    }
    await _G.emailRead(emailId); // mark-as-read is implicit
    return `Queued move to "${requestedFolder}".`;
  });

  const prompt = `
<user_instruction>
${_G.xmlEscape(instruction)}
</user_instruction>

<valid_move_folders>
${_G.xmlEscape(_G.renderMoveFolderChoicesLib(_G.cachedMoveFolders))}
</valid_move_folders>
`;

  const result = await microagent.run({ prompt });
  _G.log('microagent.result', {
    name: 'executeInstructionMicroagent',
    input: {
      emailId,
      instruction,
      validMoveFolders: _G.cachedMoveFolders,
    },
    output: result,
  }, 'microagent');
  return result;
};
