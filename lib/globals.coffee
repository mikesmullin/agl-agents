import { resolve } from 'path'

export _G =
  ENTITY_DIR: null  # override in trial mode before Entity.init()
  ARCHIVE_DIR: null  # override in trial mode
  MODEL:
    process.env.AGENT_MODEL or
    # 'xai:grok-4-1-fast-reasoning'
    # 'ollama:gemma4:26b'
    'lm-studio:google/gemma-4-e4b'
  EMAIL_BODY_MAX_BYTES: 400_000 # max bytes for filtered email body fed to any microagent (~100k tokens)
  DB_DIR: resolve process.cwd(), 'personal-email/db'
  MEMO_DB: resolve process.cwd(), 'personal-email/db/journal'
  PRESENTATION_MEMO_DB: resolve process.cwd(), 'personal-email/db/presentation'
  PAGE_SIZE: 10
  PULL_BATCH_SIZE: 10
  cachedMoveFolders: []
