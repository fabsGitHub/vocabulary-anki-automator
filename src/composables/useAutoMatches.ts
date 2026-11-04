/**
 * Previews of matching a note type's fields by name (`suggest_field_mapping`)
 * for every mapping that is `null`, so matched at push time: each such entry
 * of "My languages" (docs/architecture.md § 8.2). A preview lives here, never
 * in the form, so it dirties nothing; "Customize" copies it into the form.
 *
 * Each caller keeps its own cache, keyed by note type and language pair, so
 * two entries with the same note type and pair share one request. A pair
 * is asked once while Anki stays online; leaving `online` (Anki closed, or a
 * rescan) forgets every answer and drops the ones still on their way,
 * because a note type may have changed meanwhile. A failed match is asked
 * again once the list of requests changes.
 */

import { reactive, watch } from 'vue'

import { errorMessage, suggestFieldMapping, type AutoMatch } from '../api'
import { useAnkiConnectionContext } from './useAnkiConnection'

/** One mapping to preview: its note type and the pair cards are matched for. */
export interface AutoMatchRequest {
  model: string
  sourceLang: string
  targetLang: string
}

export interface AutoMatchState {
  /** The match; `null` while offline, while loading and after a failed match. */
  match: AutoMatch | null
  loading: boolean
  error: string | null
}

export interface AutoMatches {
  /** What is known about `request`'s match; reactive, and reading it requests nothing. */
  state(request: AutoMatchRequest): AutoMatchState
}

const UNKNOWN: AutoMatchState = Object.freeze({ match: null, loading: false, error: null })

function key(request: AutoMatchRequest): string {
  return [request.model.trim(), request.sourceLang, request.targetLang].join('\n')
}

/**
 * Keeps the match of every request `requests()` lists loaded while `online`
 * reports Anki online; a request without a note type asks nothing. Created
 * in a component's setup, or an effect scope.
 */
export function createAutoMatches(
  online: () => boolean,
  requests: () => AutoMatchRequest[],
): AutoMatches {
  const known = reactive(new Map<string, AutoMatchState>())
  /** Bumped when Anki goes offline, so an answer from before is dropped. */
  let generation = 0

  function load(request: AutoMatchRequest): void {
    const model = request.model.trim()
    const id = key(request)
    const current = known.get(id)
    if (!model || (current && (current.loading || current.match))) return
    const run = generation
    known.set(id, { match: null, loading: true, error: null })
    suggestFieldMapping(model, request.sourceLang, request.targetLang).then(
      (match) => {
        if (run === generation) known.set(id, { match, loading: false, error: null })
      },
      (err: unknown) => {
        if (run === generation) {
          known.set(id, { match: null, loading: false, error: errorMessage(err) })
        }
      },
    )
  }

  watch(
    [online, () => requests().map(key).join('\n\n')],
    ([now]) => {
      if (!now) {
        generation++
        known.clear()
        return
      }
      for (const request of requests()) load(request)
    },
    { immediate: true },
  )

  return { state: (request) => known.get(key(request)) ?? UNKNOWN }
}

/** {@link createAutoMatches} for the connection the settings window provides. */
export function useAutoMatches(requests: () => AutoMatchRequest[]): AutoMatches {
  const { state } = useAnkiConnectionContext()
  return createAutoMatches(() => state.value === 'online', requests)
}
