import { _G } from './globals.mjs';

export const ANSI = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  cyan: '\x1b[38;2;80;200;255m',
  green: '\x1b[38;2;90;220;120m',
  red: '\x1b[38;2;255;120;120m',
  amber: '\x1b[38;2;255;195;85m',
  violet: '\x1b[38;2;178;138;255m',
};

_G.ANSI = ANSI;
