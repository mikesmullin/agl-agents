import { YAML } from 'bun'
import { readFile } from 'fs/promises'
import { resolve } from 'path'

CONFIG_PATH = resolve process.cwd(), 'config.yaml'

configText = ''
try
  configText = await readFile CONFIG_PATH, 'utf8'
catch # missing config is fine; use default provider

config = YAML.parse(configText or '') ? {}
provider = String(config?.email?.provider ? 'google').toLowerCase()

if provider is 'google'
  await import './google-email.mjs'
else
  await import './outlook-email.mjs'
