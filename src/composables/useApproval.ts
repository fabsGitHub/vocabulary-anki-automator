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
 *
 * Each shown capture's note type is then checked with `check_pending_fields`,
 * in the background: AnkiConnect drops values whose field the note type lacks
 * without an error, and this is the last chance to say so. The check sends the
 * card as it would be approved, and runs again whenever a value is filled in
 * or emptied, the only edits that can change its answer. A check that answers
 * for an older capture, or after a newer check, is dropped, like a late save.
 *
 * A detected language can be switched: `rerun_pending_capture` builds the
 * card again for the chosen language from the current front, for the capture
 * on screen only (it names its `generation`, so a newer capture that has not
 * reached the window yet is left alone). Its result also arrives as
 * `translation-payload`, in either order; whichever comes first is shown, and
 * the other is a re-delivery of the same capture.
 */

import {
  computed,
  getCurrentScope,
  onScopeDispose,
  reactive,
  ref,
  watch,
  type ComputedRef,
  type Ref,
} from 'vue'

import {
  checkPendingFields,
  discardPendingPayload,
  errorMessage,
  executeCardGeneration,
  getPendingPayload,
  hideCurrentWindow,
  onCaptureError,
  onTranslationPayload,
  rerunPendingCapture,
  type Card,
  type FieldCheck,
  type PendingCapture,
  type ScriptMetadata,
  type SubmitResult,
  type UnlistenFn,
} from '../api'
import type { MessageKey } from '../i18n'

/**
 * `waiting`: nothing to approve. `editing`: a capture is shown. `switching`:
 * it is being captured again in another language. `submitting`: the approved
 * card is being saved. `saved`: it is in the local history, but Anki could
 * not be reached; only closing is left.
 */
export type ApprovalPhase = 'waiting' | 'editing' | 'switching' | 'submitting' | 'saved'

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
  /** What a push of the shown capture would drop; `null` until checked or with nothing shown. */
  fieldCheck: Ref<FieldCheck | null>
  /** The language being switched to while `phase` is `switching`. */
  switchingTo: Ref<string | null>
  /** Pinyin and HSK level when the capture is Chinese. */
  chinese: ComputedRef<Extract<ScriptMetadata, { kind: 'chinese' }> | null>
  /** The front differs from the captured one, so its TTS audio is stale. */
  frontEdited: ComputedRef<boolean>
  /** Listens for captures, then fetches the one that may already be waiting. */
  start(): Promise<void>
  /**
   * Captures the shown card's front again as `lang` (`name` is its display
   * name, for the error). Resolves to whether the switched card is on screen.
   */
  switchLanguage(lang: string, name: string): Promise<boolean>
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
  const fieldCheck = ref<FieldCheck | null>(null)
  const switchingTo = ref<string | null>(null)

  const chinese = computed(() => {
    const metadata = pending.value?.card.script_metadata
    return metadata?.kind === 'chinese' ? metadata : null
  })
  const frontEdited = computed(
    () => pending.value !== null && draft.front.trim() !== pending.value.card.front.trim(),
  )

  let sequence = 0
  /** Bumped by every field check, so an answer that a newer check overtook is dropped. */
  let checkSeq = 0
  /** {@link filledValues} when the fields were last checked. */
  let checkedFilled = ''
  let eventSeen = false
  let discarding = false
  let disposed = false
  const unlisteners: UnlistenFn[] = []

  function show(capture: PendingCapture | null): void {
    sequence++
    pending.value = capture
    result.value = null
    error.value = null
    fieldCheck.value = null
    switchingTo.value = null
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
      void checkFields()
    } else {
      Object.assign(draft, emptyDraft())
      phase.value = 'waiting'
    }
  }

  /** Which draft values are filled in: what a value without a field depends on. */
  function filledValues(): string {
    return [draft.front, draft.back, draft.example_source, draft.example_target, draft.notes]
      .map((value) => (value.trim() === '' ? '0' : '1'))
      .join('')
  }

  /** The card as `approve` submits it: `capture`'s card with the edited, trimmed values. */
  function draftCard(capture: PendingCapture): Card {
    return {
      ...capture.card,
      front: draft.front.trim(),
      back: draft.back.trim(),
      example_source: draft.example_source.trim(),
      example_target: draft.example_target.trim(),
      notes: draft.notes.trim(),
    }
  }

  /** Fetches the field check for the shown capture as edited; a failure only means no warning. */
  async function checkFields(): Promise<void> {
    const capture = pending.value
    if (!capture) return
    const mine = sequence
    const run = ++checkSeq
    checkedFilled = filledValues()
    let check: FieldCheck | null = null
    try {
      check = await checkPendingFields(draftCard(capture))
    } catch (err) {
      console.warn('[approval] could not check the fields of the note type:', err)
    }
    if (mine === sequence && run === checkSeq) fieldCheck.value = check
  }

  // Typing on in a filled field changes nothing Anki would drop, so Anki is
  // asked again only when a value is filled in or emptied.
  watch(filledValues, (filled) => {
    if (phase.value === 'editing' && pending.value && filled !== checkedFilled) void checkFields()
  })

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

  async function switchLanguage(lang: string, name: string): Promise<boolean> {
    const capture = pending.value
    if (phase.value !== 'editing' || !capture) return false
    // Already this language: nothing would change but the quota spent.
    if (lang === capture.card.source_lang) return true
    const mine = sequence
    phase.value = 'switching'
    switchingTo.value = lang
    error.value = null

    let switched: PendingCapture
    try {
      switched = await rerunPendingCapture(lang, draft.front, capture.generation)
    } catch (err) {
      if (mine === sequence) {
        phase.value = 'editing'
        switchingTo.value = null
        error.value = {
          key: 'approval.switchFailed',
          params: { lang: name, error: errorMessage(err) },
        }
      }
      return false
    }
    // The event may have shown it already, or a newer capture replaced it.
    if (mine !== sequence) return sameCapture(pending.value, switched)
    show(switched)
    return true
  }

  async function approve(): Promise<void> {
    const capture = pending.value
    if (phase.value !== 'editing' || !capture) return
    if (!draft.front.trim()) {
      error.value = { key: 'approval.frontRequired' }
      return
    }
    const mine = sequence
    phase.value = 'submitting'
    error.value = null
    const card = draftCard(capture)

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
    const busy = phase.value === 'switching' || phase.value === 'submitting'
    if (busy || phase.value === 'saved' || discarding) return
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
    fieldCheck,
    switchingTo,
    chinese,
    frontEdited,
    start,
    switchLanguage,
    approve,
    discard,
    close,
    dismissCaptureError: () => (captureError.value = null),
    dispose,
  }
}
