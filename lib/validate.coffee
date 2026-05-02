import { _G } from './globals.mjs'

export mustBeBooleanOr = _G.mustBeBooleanOr = (value, fallback = false) ->
  if typeof value === 'boolean' then value else Boolean(fallback)

export mustBeStringOr = _G.mustBeStringOr = (value, fallback = '') ->
  if typeof value === 'string' then value else String(fallback)

export mustBeTrimmedStringOr = _G.mustBeTrimmedStringOr = (value, fallback = '') ->
  text = if typeof value === 'string' then value else String(fallback)
  text.trim() or String(fallback).trim()

export joinStringListOr = _G.joinStringListOr = (value, delimiter = ',', fallback = '') ->
  if not Array.isArray(value) or value.length === 0 or value.some((item) -> typeof item !== 'string')
    return String(fallback)

  joined = value.join(delimiter)
  joined or String(fallback)
