import { resolve } from 'path';

export const _G = {
  MODEL:
    process.env.AGENT_MODEL ||
    // 'xai:grok-4-1-fast-reasoning'
    // 'ollama:gemma4:26b'
    'lm-studio:google/gemma-4-e4b',
  DB_DIR: resolve(process.cwd(), 'personal-email/db'),
  MEMO_DB: resolve(process.cwd(), 'personal-email/db/journal'),
  PRESENTATION_MEMO_DB: resolve(process.cwd(), 'personal-email/db/presentation'),
  PAGE_SIZE: 10,
  PULL_BATCH_SIZE: 10,
  cachedMoveFolders: [],
};
