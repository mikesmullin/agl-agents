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

  microagent.Tool('mark_as_read', 'Mark the email as read', {}, [], async () => {
    const r = await _G.spawn('google-email', ['inbox', 'read', emailId]);
    return r.code === 0 ? 'Marked as read.' : `Failed: ${r.stderr} r${r.stdout}`;
  });

  microagent.Tool('mark_as_unread', 'Mark the email as unread in offline metadata.', {}, [], async () => {
    const r = await _G.spawn('google-email', ['inbox', 'unread', emailId]);
    return r.code === 0 ? 'Marked as unread.' : `Failed: ${r.stderr} ${r.stdout}`;
  });

  microagent.Tool('delete', 'Delete the email.', {}, [], async () => {
    const del = await _G.spawn('google-email', ['delete', emailId]);
    if (del.code !== 0) {
      return `Failed to delete email: ${del.stderr} ${del.stdout}`;
    }
    return 'Deleted.';
  });

  microagent.Tool('move', 'Apply a label to the email (same as moving it into a folder). This can also archive email.', {
    folder: { type: 'string', description: 'Destination folder name. Must match existing (case-sensitive).' },
  }, ['folder'], async (ctx, { folder }) => {
    const requestedFolder = _G.mustBeTrimmedStringOr(folder, 'Unknown');

    let move;
    if (_G.stristr(requestedFolder, 'archive')) {
      move = await _G.spawn('google-email', ['archive', emailId]);
    } else {
      if (!_G.findMoveFolderLib(_G.cachedMoveFolders, requestedFolder)) {
        return _G.invalidFolderMessageLib(requestedFolder, _G.cachedMoveFolders);
      }

      move = await _G.spawn('google-email', ['move', emailId, requestedFolder]);
    }
    if (move.code !== 0) {
      return `Failed to move email: ${move.stderr} ${move.stdout}.`;
    }

    return `Moved email to "${JSON.stringify(requestedFolder)}".`;
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
