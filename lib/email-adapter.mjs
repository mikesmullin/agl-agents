import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { resolve } from 'path';

const CONFIG_PATH = resolve(process.cwd(), 'config.yaml');

let configText = '';
try {
  configText = await readFile(CONFIG_PATH, 'utf8');
} catch { /* missing config is fine; use default provider */ }

const config = YAML.parse(configText || '') || {};
const provider = String(config?.email?.provider || 'google').toLowerCase();

if (provider === 'google') {
  await import('./google-email.mjs');
} else {
  await import('./outlook-email.mjs');
}
