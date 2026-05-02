import { _G } from '../../lib/globals.coffee'

_entities = {}

_G.World =
  add: (entity) ->
    _entities[entity.id] = entity
    entity

  set: (entity) ->
    _entities[entity.id] = entity
    entity

  remove: (id) ->
    delete _entities[id]

  Entity__find: (filterFn) ->
    Object.values(_entities).filter filterFn
