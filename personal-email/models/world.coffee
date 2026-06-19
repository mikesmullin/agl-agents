import { _G } from '../../lib/globals.coffee'

_entities = {}
_entitiesArray = []

_G.World =
  get: (id) ->
    _entities[String(id)]

  set: (entity) ->
    if _entities[entity.id]
      # Update in-place to avoid rebuilding the array
      idx = _entitiesArray.findIndex (e) -> e.id is entity.id
      if idx >= 0
        _entitiesArray[idx] = entity
      else
        _entitiesArray.push entity
    else
      _entitiesArray.push entity
    _entities[entity.id] = entity
    entity

  remove: (id) ->
    if _entities[id]
      delete _entities[id]
      _entitiesArray = _entitiesArray.filter (e) -> e.id isnt id

  # Bypasses backoff filter — use for status/debug only.
  Entity__all: ->
    _entitiesArray.slice()

  Entity__find: (filterFn) ->
    now = Date.now()
    _entitiesArray.filter (e) ->
      # Skip entities in backoff until nextRetryAt has passed
      return false if e._error?.nextRetryAt? and new Date(e._error.nextRetryAt).getTime() > now
      filterFn e
