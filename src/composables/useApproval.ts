/**
 * The approval window's logic (Milestone D, the MVP gap): show the pending
 * capture, let the user fix it, then save it or throw it away.
 *
 * A capture reaches the window two ways, and both are needed. The backend
 * emits `translation-payload` to a window that already exists, but when it
 * has to create the window first, the event fires before this code listens;
 * `get_pending_payload` on start-up covers that case. The listener is
 * registered before the fetch, and an event that arrives meanwhile wins,
 * because it is the newer capture.
 *
 * Every shown capture gets a sequence number. A save that resolves after a
 * newer capture arrived must not hide the window or clear the form, since
 * both belong to the new card by then.
 */

import {
  computed,
  getCurrentScope,
  onScopeDispose,
  reactive,
  ref,
  type ComputedRef,
  type Ref,
} from 'vue'

import {
  discardPendingPayload,
  errorMessage,
  executeCardGeneration,
  getPendingPayload,
  hideCurrentWindow,
  onCaptureError,
  onTranslationPayload,
  type Card,
  type PendingCapture,
  type ScriptMetadata,
  type SubmitResult,
  type UnlistenFn,
} from '../api'
import type { MessageKey } from '../i18n'

/**
 * `waiting`: nothing to approve. `editing`: a capture is shown. `submitting`:
 * the approved card is being saved. `saved`: it is in the local history, but
 * Anki could not be reached; only closing is left.
 */
export type ApprovalPhase = 'waiting' | 'editing' | 'submitting' | 'saved'

/** The editable part of the card. */
export interface CardDraft {
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
}

export interface ApprovalError {
  key: MessageKey
  params?: Record<string, string | number>
}

export interface Approval {
  pending: Ref<PendingCapture | null>
  draft: CardDraft
  phase: Ref<ApprovalPhase>
  /** The save result while `phase` is `saved`. */
  result: Ref<SubmitResult | null>
  error: Ref<ApprovalError | null>
  /** The last `capture-error` message, until the next capture arrives. */
  captureError: Ref<string | null>
  /** Pinyin and HSK level when the capture is Chinese. */
  chinese: ComputedRef<Extract<ScriptMetadata, { kind: 'chinese' }> | null>
  /** The front differs from the captured one, so its TTS audio is stale. */
  frontEdited: ComputedRef<boolean>
  /** Listens for captures, then fetches the one that may already be waiting. */
  start(): Promise<void>
  /** Saves the edited card; hides the window when Anki took it. */
  approve(): Promise<void>
  /** Drops the pending capture; the backend hides the window. */
  discard(): Promise<void>
  /** Hides the window after an offline save. */
  close(): Promise<void>
  dismissCaptureError(): void
  dispose(): void
}

function emptyDraft(): CardDraft {
  return { front: '', back: '', example_source: '', example_target: '', notes: '' }
}

/** Exactly the same payload; anything that differs is a capture the user must see. */
function sameCapture(a: PendingCapture | null, b: PendingCapture): boolean {
  return a !== null && JSON.stringify(a) === JSON.stringify(b)
}

export function useApproval(): Approval {
  const pending = ref<PendingCapture | null>(null)
  const draft = reactive<CardDraft>(emptyDraft())
  const phase = ref<ApprovalPhase>('waiting')
  const result = ref<SubmitResult | null>(null)
  const error = ref<ApprovalError | null>(null)
  const captureError = ref<string | null>(null)

  const chinese = computed(() => {
    const metadata = pending.value?.card.script_metadata
    return metadata?.kind === 'chinese' ? metadata : null
  })
  const frontEdited = computed(
    () => pending.value !== null && draft.front.trim() !== pending.value.card.front.trim(),
  )

  let sequence = 0
  let eventSeen = false
  let discarding = false
  let disposed = false
  const unlisteners: UnlistenFn[] = []

  function show(capture: PendingCapture | null): void {
    sequence++
    pending.value = capture
    result.value = null
    error.value = null
    if (capture) {
      captureError.value = null
      Object.assign(draft, {
        front: capture.card.front,
        back: capture.card.back,
        example_source: capture.card.example_source,
        example_target: capture.card.example_target,
        notes: capture.card.notes,
      })
      phase.value = 'editing'
    } else {
      Object.assign(draft, emptyDraft())
      phase.value = 'waiting'
    }
  }

  /** A re-delivery of the capture on screen keeps the user's edits (and a finished save). */
  function receive(capture: PendingCapture): void {
    if (!sameCapture(pending.value, capture)) show(capture)
  }

  function stopListening(): void {
    for (const unlisten of unlisteners.splice(0)) unlisten()
  }

  async function start(): Promise<void> {
    try {
      unlisteners.push(
        await onTranslationPayload((capture) => {
          eventSeen = true
          receive(capture)
        }),
      )
      unlisteners.push(await onCaptureError((payload) => (captureError.value = payload.message)))
    } catch (err) {
      console.error('[approval] could not listen for captures:', err)
    }
    if (disposed) {
      stopListening()
      return
    }
    try {
      const capture = await getPendingPayload()
      if (capture && !eventSeen) receive(capture)
    } catch (err) {
      console.error('[approval] get_pending_payload failed:', err)
    }
  }

  async function hide(): Promise<void> {
    try {
      await hideCurrentWindow()
    } catch (err) {
      console.error('[approval] could not hide the window:', err)
    }
  }

  async function approve(): Promise<void> {
    const capture = pending.value
    if (phase.value !== 'editing' || !capture) return
    const front = draft.front.trim()
    if (!front) {
      error.value = { key: 'approval.frontRequired' }
      return
    }
    const mine = sequence
    phase.value = 'submitting'
    error.value = null
    const card: Card = {
      ...capture.card,
      front,
      back: draft.back.trim(),
      example_source: draft.example_source.trim(),
      example_target: draft.example_target.trim(),
      notes: draft.notes.trim(),
    }

    let outcome: SubmitResult
    try {
      outcome = await executeCardGeneration(card)
    } catch (err) {
      if (mine === sequence) {
        phase.value = 'editing'
        error.value = { key: 'approval.submitFailed', params: { error: errorMessage(err) } }
      }
      return
    }
    if (mine !== sequence) return
    if (outcome.synced_to_anki) {
      await hide()
      if (mine === sequence) show(null)
    } else {
      result.value = outcome
      phase.value = 'saved'
    }
  }

  async function discard(): Promise<void> {
    if (phase.value === 'submitting' || phase.value === 'saved' || discarding) return
    discarding = true
    const mine = sequence
    try {
      await discardPendingPayload()
      if (mine === sequence) show(null)
    } catch (err) {
      error.value = { key: 'approval.discardFailed', params: { error: errorMessage(err) } }
    } finally {
      discarding = false
    }
  }

  async function close(): Promise<void> {
    if (phase.value !== 'saved') return
    const mine = sequence
    await hide()
    if (mine === sequence) show(null)
  }

  function dispose(): void {
    disposed = true
    stopListening()
  }

  if (getCurrentScope()) onScopeDispose(dispose)

  return {
    pending,
    draft,
    phase,
    result,
    error,
    captureError,
    chinese,
    frontEdited,
    start,
    approve,
    discard,
    close,
    dismissCaptureError: () => (captureError.value = null),
    dispose,
  }
}
