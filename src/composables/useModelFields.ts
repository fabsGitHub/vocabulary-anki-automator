/**
 * The fields of Anki note types, read through `anki_model_field_names` and
 * shared by the settings tabs: the Anki tab and "My languages" usually ask
 * for the same note type, and every entry of "My languages" may name one.
 *
 * The main window provides one cache keyed by note type. Each note type is
 * read once while Anki stays online, and two callers asking at the same time
 * share the request. Leaving `online` (Anki closed, or a rescan) forgets every
 * answer and drops the ones still on their way, because a note type may have
 * changed meanwhile. A failed read is kept until the next `load` asks again.
 */

import { inject, provide, reactive, watch, type InjectionKey } from 'vue'

import { ankiModelFieldNames, errorMessage } from '../api'
import { useAnkiConnectionContext } from './useAnkiConnection'

export interface ModelFieldsState {
  /** The note type's fields; `null` while offline, while loading and after a failed read. */
  fields: string[] | null
  loading: boolean
  error: string | null
}

export interface ModelFields {
  /** What is known about `model`'s fields; reactive, and reading it requests nothing. */
  state(model: string): ModelFieldsState
  /** Reads `model`'s fields unless they are known or on their way; nothing while offline. */
  load(model: string): void
}

const UNKNOWN: ModelFieldsState = Object.freeze({ fields: null, loading: false, error: null })

/** A cache for the connection whose state `online` reports; created in a component's setup. */
export function createModelFields(online: () => boolean): ModelFields {
  const known = reactive(new Map<string, ModelFieldsState>())
  /** Bumped when Anki goes offline, so an answer from before is dropped. */
  let generation = 0

  watch(
    online,
    (now) => {
      if (now) return
      generation++
      known.clear()
    },
    { flush: 'sync' },
  )

  function load(model: string): void {
    const name = model.trim()
    if (!online() || !name) return
    const current = known.get(name)
    if (current && (current.loading || current.fields)) return
    const run = generation
    known.set(name, { fields: null, loading: true, error: null })
    ankiModelFieldNames(name).then(
      (fields) => {
        if (run === generation) known.set(name, { fields, loading: false, error: null })
      },
      (err: unknown) => {
        if (run === generation) {
          known.set(name, { fields: null, loading: false, error: errorMessage(err) })
        }
      },
    )
  }

  return {
    state: (model) => known.get(model.trim()) ?? UNKNOWN,
    load,
  }
}

/** Injection key of the main window's cache. */
export const MODEL_FIELDS_KEY: InjectionKey<ModelFields> = Symbol('model-fields')

/** Shares `fields` with every settings tab below the calling component. */
export function provideModelFields(fields: ModelFields): void {
  provide(MODEL_FIELDS_KEY, fields)
}

/**
 * The provided cache, kept loaded with every note type `models()` names
 * while Anki is online. A tab mounted on its own (a test) gets a cache of
 * its own for the connection it is given.
 */
export function useModelFields(models: () => string[]): ModelFields {
  const { state } = useAnkiConnectionContext()
  const online = () => state.value === 'online'
  const cache = inject(MODEL_FIELDS_KEY, null) ?? createModelFields(online)
  watch(
    [online, () => models().join('\n')],
    () => {
      for (const model of models()) cache.load(model)
    },
    { immediate: true },
  )
  return cache
}
