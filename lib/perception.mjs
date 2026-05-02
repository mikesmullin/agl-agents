/**
 * lib/perception.mjs
 *
 * Lightweight client for the perception-voice Unix socket server.
 * Mirrors the IPC protocol used by whisper/perception_client.py:
 *   - 4-byte big-endian length prefix + JSON payload
 *   - Commands: { command: "set", uid } and { command: "get", uid }
 *
 * Also implements the same word-mapping normalization and pause-buffer
 * logic as whisper's VoiceKeyboard / KeyboardTyper, so spoken phrases
 * are resolved to their mapped values before being returned to callers.
 */

import net from 'net';
import { YAML } from 'bun';
import { readFile } from 'fs/promises';
import { resolve } from 'path';

const HEADER_SIZE = 4;
const UID = 'personal-email-agent';
const POLL_INTERVAL_MS = 300;
const PAUSE_THRESHOLD_S = 0.8;  // same default as whisper config

// ---------------------------------------------------------------------------
// Normalize a phrase for exact-match lookup (alpha only, single spaces)
// ---------------------------------------------------------------------------
function normalize(text) {
  return text.toLowerCase().replace(/[^a-z]+/g, ' ').replace(/\s+/g, ' ').trim();
}

// ---------------------------------------------------------------------------
// Low-level socket helpers (mirrors perception_client.py framing)
// ---------------------------------------------------------------------------
async function socketRequest(socketPath, message) {
  return new Promise((resolve, reject) => {
    const sock = net.createConnection(socketPath);
    const chunks = [];
    let headerBuf = null;
    let expectedLen = 0;
    let received = 0;

    sock.on('error', reject);

    sock.on('data', (chunk) => {
      chunks.push(chunk);
      received += chunk.length;
      const buf = Buffer.concat(chunks);

      if (!headerBuf && buf.length >= HEADER_SIZE) {
        expectedLen = buf.readUInt32BE(0);
        headerBuf = true;
      }

      if (headerBuf && buf.length >= HEADER_SIZE + expectedLen) {
        sock.destroy();
        try {
          const payload = buf.slice(HEADER_SIZE, HEADER_SIZE + expectedLen).toString('utf8');
          resolve(JSON.parse(payload));
        } catch (e) {
          reject(e);
        }
      }
    });

    sock.on('connect', () => {
      const payload = Buffer.from(JSON.stringify(message), 'utf8');
      const header = Buffer.allocUnsafe(HEADER_SIZE);
      header.writeUInt32BE(payload.length, 0);
      sock.write(Buffer.concat([header, payload]));
    });
  });
}

async function setReadMarker(socketPath) {
  try {
    const resp = await socketRequest(socketPath, { command: 'set', uid: UID });
    return resp?.status === 'ok';
  } catch { return false; }
}

async function getTranscriptions(socketPath) {
  try {
    const resp = await socketRequest(socketPath, { command: 'get', uid: UID });
    if (resp?.status !== 'ok' || !resp.text) return [];
    return resp.text.trim().split('\n').filter(Boolean).map((line) => {
      try { return JSON.parse(line); } catch { return null; }
    }).filter(Boolean);
  } catch { return []; }
}

// ---------------------------------------------------------------------------
// PerceptionInput — wraps everything into a readline-compatible ask()
// ---------------------------------------------------------------------------
export class PerceptionInput {
  constructor({ socketPath, wordMappings = {}, pauseThresholdS = PAUSE_THRESHOLD_S }) {
    this.socketPath = socketPath;
    this.pauseThresholdS = pauseThresholdS;
    this._cancelActiveQuestion = null;
    // Build normalized map
    this._mappings = {};
    for (const [phrase, output] of Object.entries(wordMappings)) {
      this._mappings[normalize(phrase)] = output;
    }
  }

  /** Resolve a buffered utterance string to its mapped value, or null. */
  _resolve(text) {
    return this._mappings[normalize(text)] ?? null;
  }

  /**
   * Set the read marker so future get() calls only return new utterances.
   * Call this just before displaying a prompt.
   */
  async setMarker() {
    await setReadMarker(this.socketPath);
  }

  cancelActiveQuestion() {
    if (typeof this._cancelActiveQuestion === 'function') {
      this._cancelActiveQuestion();
    }
  }

  /**
   * Display `promptText` to stdout, then poll perception-voice until a
   * word-mapped utterance arrives.  Returns the mapped value (e.g. "proceed\n").
   *
   * While polling, any unmapped utterance is printed as a hint so the operator
   * can see what was heard but not matched.
   */
  async question(promptText, rl) {
    this.cancelActiveQuestion();

    process.stdout.write(promptText);
    await setReadMarker(this.socketPath);

    let pendingTexts = [];
    let pendingLastTs = null;
    let spokenBuffer = '';
    let settled = false;
    let polling = false;

    const appendSpoken = (text) => {
      const trimmed = String(text || '').trim();
      if (!trimmed) return;
      spokenBuffer = spokenBuffer ? `${spokenBuffer} ${trimmed}` : trimmed;
    };
    const flushPending = () => {
      if (!pendingTexts.length) return '';
      const buffered = pendingTexts.join(' ').trim();
      pendingTexts = [];
      pendingLastTs = null;
      return buffered;
    };
    const refreshPrompt = () => {
      process.stdout.write(promptText);
    };
    const emitHeard = (text, suffix = '') => {
      process.stdout.write(`\n  [heard: "${text}"${suffix}]\n`);
    };

    return new Promise((resolvePromise, rejectPromise) => {
      const controller = new AbortController();
      let timer = null;
      const cancelledError = () => Object.assign(new Error('Perception prompt cancelled'), { code: 'ABORT_ERR' });
      const isPromptUnavailable = () => settled || controller.signal.aborted || rl?.closed;

      const cleanup = () => {
        if (timer) {
          clearInterval(timer);
          timer = null;
        }
        controller.abort();
        if (this._cancelActiveQuestion === cancel) {
          this._cancelActiveQuestion = null;
        }
      };

      const resolveWith = (value) => {
        if (settled) return;
        settled = true;
        cleanup();
        resolvePromise(value);
      };

      const rejectWith = (error) => {
        if (settled) return;
        settled = true;
        cleanup();
        rejectPromise(error);
      };

      const combineInput = (typedText) => {
        const typed = String(typedText || '');
        if (!spokenBuffer) return typed.trim();
        if (!typed) return spokenBuffer.trim();
        if (spokenBuffer.endsWith('\n') || typed.startsWith('\n')) {
          return `${spokenBuffer}${typed}`.trim();
        }
        return `${spokenBuffer} ${typed}`.trim();
      };

      const startKeyboardQuestion = () => {
        if (isPromptUnavailable()) {
          rejectWith(cancelledError());
          return;
        }
        rl.question('', { signal: controller.signal })
          .then(finalizeInput)
          .catch((error) => {
            if (error?.name === 'AbortError' || error?.code === 'ABORT_ERR') {
              rejectWith(cancelledError());
              return;
            }
            if (error?.code === 'ERR_USE_AFTER_CLOSE' && (rl?.closed || controller.signal.aborted)) {
              rejectWith(cancelledError());
              return;
            }
            rejectWith(error);
          });
      };

      const finalizeInput = (typedText) => {
        const pending = flushPending();
        if (pending) {
          emitHeard(pending);
          appendSpoken(pending);
        }
        const finalInput = combineInput(typedText);
        if (!finalInput) {
          if (isPromptUnavailable()) {
            rejectWith(cancelledError());
            return;
          }
          refreshPrompt();
          startKeyboardQuestion();
          return;
        }
        resolveWith(finalInput);
      };

      const handleFlushedUtterance = (buffered) => {
        if (!buffered) return;

        const normalized = normalize(buffered);
        const mapped = this._resolve(buffered);
        if (mapped !== null) {
          const mappedValue = mapped.replace(/\\n/g, '\n');
          if (normalized === 'erase') {
            spokenBuffer = '';
            pendingTexts = [];
            pendingLastTs = null;
            try { rl?.write(null, { ctrl: true, name: 'u' }); } catch { }
            emitHeard(buffered, ' - cleared input');
            refreshPrompt();
            return;
          }
          if (normalized === 'submit') {
            emitHeard(buffered, ' - submit');
            finalizeInput('');
            return;
          }
          if (mappedValue === '\n') {
            emitHeard(buffered);
            spokenBuffer = spokenBuffer ? `${spokenBuffer}\n` : '\n';
            refreshPrompt();
            return;
          }

          process.stdout.write(`${buffered}\n`);
          resolveWith(mappedValue.trimEnd());
          return;
        }

        emitHeard(buffered);
        appendSpoken(buffered);
        refreshPrompt();
      };

      const cancel = () => {
        rejectWith(cancelledError());
      };
      this._cancelActiveQuestion = cancel;

      startKeyboardQuestion();

      timer = setInterval(async () => {
        if (settled || polling) return;
        polling = true;
        try {
          const items = await getTranscriptions(this.socketPath);

          for (const item of items) {
            const text = String(item.text || '').trim();
            if (!text) continue;

            let itemTs;
            try { itemTs = new Date(item.ts).getTime() / 1000; } catch { itemTs = Date.now() / 1000; }

            if (pendingLastTs !== null && itemTs - pendingLastTs >= this.pauseThresholdS) {
              handleFlushedUtterance(flushPending());
              if (settled) return;
            }

            pendingTexts.push(text);
            pendingLastTs = itemTs;
          }

          if (pendingLastTs !== null && Date.now() / 1000 - pendingLastTs >= this.pauseThresholdS) {
            handleFlushedUtterance(flushPending());
          }
        }
        finally {
          polling = false;
        }
      }, POLL_INTERVAL_MS);
    });
  }
}

// ---------------------------------------------------------------------------
// Load perception config from personal-email/config.yaml under perception: key
// ---------------------------------------------------------------------------
export async function loadPerceptionConfig(cwd) {
  const configPath = resolve(cwd, 'personal-email', 'config.yaml');
  const text = await readFile(configPath, 'utf8');
  const file = YAML.parse(text) || {};
  const cfg = (file.perception && typeof file.perception === 'object') ? file.perception : {};
  const socketPath = resolve(cwd, String(cfg.socket_path || 'perception.sock'));
  const wordMappings = cfg.word_mappings && typeof cfg.word_mappings === 'object'
    ? cfg.word_mappings
    : {};
  const pauseThresholdS = Number(cfg.pause_threshold_s ?? PAUSE_THRESHOLD_S);
  return new PerceptionInput({ socketPath, wordMappings, pauseThresholdS });
}
