import { _G } from './globals.mjs';

export const mustBeBooleanOr = _G.mustBeBooleanOr = (value, fallback = false) => {
  return typeof value === 'boolean' ? value : Boolean(fallback);
};

export const mustBeStringOr = _G.mustBeStringOr = (value, fallback = '') => {
  return typeof value === 'string' ? value : String(fallback);
};

export const mustBeTrimmedStringOr = _G.mustBeTrimmedStringOr = (value, fallback = '') => {
  const text = typeof value === 'string' ? value : String(fallback);
  return text.trim() || String(fallback).trim();
};

export const joinStringListOr = _G.joinStringListOr = (value, delimiter = ',', fallback = '') => {
  if (!Array.isArray(value) || value.length === 0 || value.some((item) => typeof item !== 'string')) {
    return String(fallback);
  }

  const joined = value.join(delimiter);
  return joined || String(fallback);
};