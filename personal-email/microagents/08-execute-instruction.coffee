import Agent from '../../node_modules/agl-ai/src/agent.mjs'
import { _G } from '../../lib/globals.mjs'

export executeInstructionMicroagent = _G.executeInstructionMicroagent = (emailId, instruction) ->
  microagent = await Agent.factory
    system_prompt: 'You perform email operations on behalf of the user.'
    output_tool:
      description: 'Outcome of executing the user instruction for this email.'
      parameters:
        success:
          type: 'boolean'
          description: 'You were successful?'
        summary:
          type: 'string'
          description: 'Short summary of the action(s) you took.'
      required: ['success', 'summary']

  microagent.Tool 'mark_as_read', 'Mark the email as read (offline metadata).', {}, [], ->
    r = await _G.emailRead emailId
    if r.code is 0 then 'Marked as read.' else "Failed: #{r.stderr} #{r.stdout}"

  microagent.Tool 'mark_as_unread', 'Mark the email as unread (offline metadata).', {}, [], ->
    r = await _G.emailUnread emailId
    if r.code is 0 then 'Marked as unread.' else "Failed: #{r.stderr} #{r.stdout}"

  microagent.Tool 'delete', 'Queue the email for deletion (moved to Deleted Items when applied).', {}, [], ->
    del = await _G.emailDelete emailId
    if del.code isnt 0
      return "Failed to queue deletion: #{del.stderr} #{del.stdout}"
    await _G.emailRead emailId # mark-as-read is implicit
    'Queued for deletion.'

  microagent.Tool 'move', 'Queue the email to be moved to a folder (including Archive).',
    folder:
      type: 'string'
      description: 'Destination folder name. Must match existing (case-sensitive).'
  , ['folder'], (ctx, { folder }) ->
    requestedFolder = _G.mustBeTrimmedStringOr folder, 'Unknown'
    move = await _G.emailMove emailId, requestedFolder
    if move.code isnt 0
      return [move.stderr, move.stdout].filter(Boolean).join(' ') or 'Failed to queue move.'
    await _G.emailRead emailId # mark-as-read is implicit
    `Queued move to "#{requestedFolder}".`

  prompt = """
    <user_instruction>
    #{_G.xmlEscape instruction}
    </user_instruction>

    <valid_move_folders>
    #{_G.xmlEscape _G.renderMoveFolderChoicesLib _G.cachedMoveFolders}
    </valid_move_folders>
    """

  result = await microagent.run { prompt }
  _G.log 'microagent.result',
    name: 'executeInstructionMicroagent'
    input:
      emailId
      instruction
      validMoveFolders: _G.cachedMoveFolders
    output: result
  , 'microagent'
  result
