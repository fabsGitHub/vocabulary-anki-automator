/**
 * The local capture history (Milestone F): the latest captures, deleting one,
 * and pushing the ones Anki never received.
 *
 * Every approved card is saved locally before it goes to Anki, so the history
 * is also the retry queue for cards approved while Anki was closed. The list
 * reloads on `captures-changed`, which the backend emits after every save,
 * delete and sync, whichever window caused it; requests are numbered so an
 * older, slower load never replaces a newer one.
 */

import { getCurrentScope, onScopeDispose, ref, type Ref } from 'vue'

import {
  deleteCapture,
  errorMessage,
  listRecentCaptures,
  onCapturesChanged,
  syncPendingCaptures,
  type StoredCapture,
  type SyncReport,
  type UnlistenFn,
} from '../api'
import type { MessageKey } from '../i18n'

/** How many captures the history shows. */
export const HISTORY_LIMIT = 50

export interface HistoryNotice {
  tone: 'success' | 'error'
  key: MessageKey
  params?: Record<string, string | number>
}

export interface History {
  items: Ref<StoredCapture[]>
  loading: Ref<boolean>
  loadError: Ref<string | null>
  /** The capture whose delete confirmation is showing. */
  confirmingId: Ref<number | null>
  deletingId: Ref<number | null>
  syncing: Ref<boolean>
  /** The last sync's report, with the fronts of the failed captures resolved. */
  syncReport: Ref<(SyncReport & { failedFronts: Record<number, string> }) | null>
  notice: Ref<HistoryNotice | null>
  refresh(): Promise<void>
  askDelete(id: number): void
  cancelDelete(): void
  confirmDelete(id: number): Promise<void>
  syncPending(): Promise<void>
  /** Starts listening for `captures-changed` and loads the list. */
  start(): Promise<void>
  dispose(): void
}

export function useHistory(limit: number = HISTORY_LIMIT): History {
  const items = ref<StoredCapture[]>([])
  const loading = ref(false)
  const loadError = ref<string | null>(null)
  const confirmingId = ref<number | null>(null)
  const deletingId = ref<number | null>(null)
  const syncing = ref(false)
  const syncReport = ref<(SyncReport & { failedFronts: Record<number, string> }) | null>(null)
  const notice = ref<HistoryNotice | null>(null)

  let request = 0
  let unlisten: UnlistenFn | null = null
  let disposed = false

  async function refresh(): Promise<void> {
    const mine = ++request
    loading.value = true
    try {
      const list = await listRecentCaptures(limit)
      if (mine !== request) return
      items.value = list
      loadError.value = null
    } catch (err) {
      if (mine === request) loadError.value = errorMessage(err)
    } finally {
      if (mine === request) loading.value = false
    }
  }

  function askDelete(id: number): void {
    notice.value = null
    confirmingId.value = id
  }

  function cancelDelete(): void {
    confirmingId.value = null
  }

  async function confirmDelete(id: number): Promise<void> {
    if (deletingId.value !== null) return
    deletingId.value = id
    try {
      const removed = await deleteCapture(id)
      items.value = items.value.filter((item) => item.id !== id)
      notice.value = removed ? null : { tone: 'error', key: 'history.alreadyDeleted' }
    } catch (err) {
      notice.value = {
        tone: 'error',
        key: 'history.deleteFailed',
        params: { error: errorMessage(err) },
      }
    } finally {
      deletingId.value = null
      confirmingId.value = null
    }
    await refresh()
  }

  async function syncPending(): Promise<void> {
    if (syncing.value) return
    syncing.value = true
    notice.value = null
    syncReport.value = null
    const fronts = new Map(items.value.map((item) => [item.id, item.card.front]))
    try {
      const report = await syncPendingCaptures()
      const failedFronts: Record<number, string> = {}
      for (const failure of report.failed)
        failedFronts[failure.id] = fronts.get(failure.id) ?? `#${failure.id}`
      syncReport.value = { ...report, failedFronts }
    } catch (err) {
      notice.value = {
        tone: 'error',
        key: 'history.syncFailed',
        params: { error: errorMessage(err) },
      }
    } finally {
      syncing.value = false
    }
    await refresh()
  }

  async function start(): Promise<void> {
    try {
      const stop = await onCapturesChanged(() => void refresh())
      if (disposed) stop()
      else unlisten = stop
    } catch (err) {
      console.warn('[history] could not listen for captures-changed:', err)
    }
    await refresh()
  }

  function dispose(): void {
    disposed = true
    unlisten?.()
    unlisten = null
  }

  if (getCurrentScope()) onScopeDispose(dispose)

  return {
    items,
    loading,
    loadError,
    confirmingId,
    deletingId,
    syncing,
    syncReport,
    notice,
    refresh,
    askDelete,
    cancelDelete,
    confirmDelete,
    syncPending,
    start,
    dispose,
  }
}
