import { _G } from './globals.coffee'

export mustBeBooleanOr = (value, fallback = false) ->
  if typeof value is 'boolean' then value else Boolean(fallback)
_G.mustBeBooleanOr = mustBeBooleanOr

export mustBeStringOr = (value, fallback = '') ->
  if typeof value is 'string' then value else String(fallback)
_G.mustBeStringOr = mustBeStringOr

export mustBeTrimmedStringOr = (value, fallback = '') ->
  text = if typeof value is 'string' then value else String(fallback)
  text.trim() or String(fallback).trim()
_G.mustBeTrimmedStringOr = mustBeTrimmedStringOr

export joinStringListOr = (value, delimiter = ',', fallback = '') ->
  if not Array.isArray(value) or value.length is 0 or value.some((item) -> typeof item isnt 'string')
    return String(fallback)
  joined = value.join(delimiter)
  joined or String(fallback)
_G.joinStringListOr = joinStringListOr
