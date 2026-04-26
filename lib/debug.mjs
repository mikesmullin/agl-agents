import { ANSI } from './color.mjs';
import { _G } from './globals.mjs';
import { appendFileSync, writeFileSync } from 'fs';
import { resolve } from 'path';
import { inspect } from 'util';

const DEBUG = /^(1|true|yes|on)$/i.test(process.env.DEBUG || '');
export const HUMAN_TRACE = process.env.DEBUG !== '1';
const DEBUG_LOG_PATH = resolve(process.cwd(), 'debug.log');
let activeTrace = null;

if (DEBUG) {
  writeFileSync(DEBUG_LOG_PATH, '');
}

function formatLogData(data) {
  if (typeof data === 'undefined') {
    return '';
  }

  if (typeof data === 'string') {
    return data;
  }

  return inspect(data, {
    depth: null,
    colors: false,
    maxArrayLength: null,
    maxStringLength: null,
    breakLength: 120,
    compact: false,
    sorted: true,
  });
}

export const debugLog = _G.debugLog = (scope, message, data) => {
  if (!DEBUG) return;
  const prefix = `[${new Date().toISOString()}] [${scope}] ${message}`;
  const suffix = formatLogData(data);
  const entry = suffix ? `${prefix}\n${suffix}\n` : `${prefix}\n`;
  appendFileSync(DEBUG_LOG_PATH, entry);
};

export const log = _G.log = (message, data, scope = 'agent') => {
  debugLog(scope, message, data);
};

export const completedLabel = _G.completedLabel = (label) => {
  const map = {
    'Pulling ': 'Pulled ',
    'Loading ': 'Loaded ',
    'Refreshing ': 'Refreshed ',
    'Reloading ': 'Reloaded ',
    'Searching ': 'Searched ',
    'Extracting ': 'Extracted ',
    'Pre-filtering ': 'Pre-filtered ',
    'Summarizing ': 'Summarized ',
    'Generating ': 'Generated ',
    'Checking ': 'Checked ',
    'Analyzing ': 'Analyzed ',
    'Executing ': 'Executed ',
    'Planning ': 'Planned ',
    'Applying ': 'Applied ',
    'Cleaning ': 'Cleaned ',
    'Building ': 'Built ',
    'Saving ': 'Saved ',
  };

  for (const prefix in map) {
    if (label.startsWith(prefix)) {
      return `${map[prefix]}${label.slice(prefix.length)}.`;
    }
  }

  return `${label} done.`;
};

export const traceReplaceLine = _G.traceReplaceLine = (emoji, text, { final = false, failed = false, ms = null } = {}) => {
  if (!HUMAN_TRACE) return;

  const color = failed ? ANSI.red : ANSI.cyan;
  const detail = ms == null ? '' : ` ${ANSI.dim}(${ms} ms)${ANSI.reset}`;
  const line = `${color}${ANSI.bold}${emoji}${ANSI.reset} ${color}${text}${ANSI.reset}${detail}`;

  if (process.stdout?.isTTY) {
    const prefix = '\r\x1b[2K';
    process.stdout.write(final ? `${prefix}${line}\n` : `${prefix}${line}`);
    return;
  }

  if (final) {
    console.log(line);
  }
};

export const printRobotAnswer = _G.printRobotAnswer = (answer) => {
  const text = String(answer || '').trim() || 'I could not determine an answer from this email.';
	/*await*/ _G.speakText(text); // speak (backgrounded, parallelized)
  console.log(`${ANSI.cyan}${ANSI.bold}🗣️ Answer:${ANSI.reset} ${ANSI.cyan}${text}${ANSI.reset}`);
};

export const traceStart = _G.traceStart = (emoji, label) => {
  const trace = {
    emoji,
    label,
    started: Date.now(),
    ended: false,
    traceEnd() {
      if (this.ended || !HUMAN_TRACE) return;
      this.ended = true;
      if (activeTrace === this) {
        activeTrace = null;
      }
      traceReplaceLine(this.emoji, completedLabel(this.label), {
        final: true,
        ms: Date.now() - this.started,
      });
    },
    traceFail() {
      if (this.ended || !HUMAN_TRACE) return;
      this.ended = true;
      if (activeTrace === this) {
        activeTrace = null;
      }
      traceReplaceLine(this.emoji, `${this.label} failed.`, {
        final: true,
        failed: true,
        ms: Date.now() - this.started,
      });
    },
  };

  if (!HUMAN_TRACE) return trace;
  activeTrace = trace;
  traceReplaceLine(emoji, `${label}...`);
  return trace;
};

export const traceEnd = _G.traceEnd = () => {
  activeTrace?.traceEnd();
};

export const traceFail = _G.traceFail = () => {
  activeTrace?.traceFail();
};

export const traceStep = _G.traceStep = async (emoji, label, fn) => {
  const trace = traceStart(emoji, label);
  try {
    const result = await fn();
    trace.traceEnd();
    return result;
  }
  catch (err) {
    trace.traceFail();
    throw err;
  }
};
_G.HUMAN_TRACE = HUMAN_TRACE;
