/**
 * The live demo (docs/live-demo.md): Learn's web build on a database of its
 * own, started with a made-up history and a clock the visitor can move.
 *
 * - **Its own database**, `vocab-learn-demo`, so a demo hosted next to the
 *   real web build (one origin, as on a GitHub Pages user site) never
 *   touches anybody's own words. Where the browser refuses IndexedDB (some
 *   private windows, storage blocked for embedded pages) the demo runs in
 *   memory instead and says that a reload starts it over.
 * - **The history is stored in one transaction**, together with the whole
 *   curriculum and the demo's own `app_state` keys, so a demo is either
 *   prepared completely or not at all. It is prepared again when it is
 *   missing or a week old, and when the visitor resets it.
 * - **"Next day" moves the demo's clock**, not the data: the app reads one
 *   clock shifted by whole days, for the core and for the UI alike, and the
 *   shift is kept with the demo's data. Each move or reset remounts the app
 *   (`epoch`), so every view reloads from the store as on a fresh start.
 */

import { type InjectionKey, type Ref, inject, ref } from 'vue'

import type { Clock as UiClock } from '../context'
import { errorMessage } from '../domain/errors'
import type { IsoDate, PlatformAdapter } from '../platform/types'
import { WebAdapter } from '../platform/web/adapter'
import { loadBundledCurriculum } from '../platform/web/curriculum'
import { openIndexedDbStorage } from '../platform/web/idb'
import type { CurriculumItem } from '../platform/web/model'
import { MemoryStorage, TABLES, type Storage } from '../platform/web/storage'
import { Store } from '../platform/web/store'
import {
  type Clock,
  MS_PER_DAY,
  daysBetween,
  localDate,
  nanosFromMillis,
} from '../platform/web/time'
import { playHistory } from './history'

/** The demo's IndexedDB database, apart from the web build's `vocab-learn`. */
export const DEMO_DATABASE = 'vocab-learn-demo'
/** `app_state`: the local date the stored history was prepared for. */
export const PREPARED_KEY = 'demo.prepared_for'
/** `app_state`: how many days "Next day" has moved the clock. */
export const DAY_SHIFT_KEY = 'demo.day_shift'
/** A history this many days old is replaced by a fresh one. */
export const STALE_AFTER_DAYS = 7

/** What the demo's bar can do, and what it shows. */
export interface DemoController {
  /** Bumped whenever the app must remount: after a move or a reset. */
  readonly epoch: Ref<number>
  /** Days the clock is ahead of the real one. */
  readonly dayShift: Ref<number>
  /** Whether the demo is kept in IndexedDB (or only in memory). */
  readonly persistent: boolean
  readonly busy: Ref<boolean>
  readonly error: Ref<string | null>
  /** The demo's "now", in milliseconds. */
  now(): number
  /** Moves the clock one day ahead. */
  nextDay(): Promise<void>
  /** Starts over with a fresh history and the real clock. */
  reset(): Promise<void>
}

/** Where the demo's root provides its controller to the demo's bar. */
export const DEMO_KEY: InjectionKey<DemoController> = Symbol('learn-demo')

export function useDemo(): DemoController {
  const demo = inject(DEMO_KEY, null)
  if (!demo) throw new Error("The demo's components need its controller (DEMO_KEY)")
  return demo
}

export interface Demo {
  platform: PlatformAdapter
  /** The UI's clock, the same shifted clock as the platform's. */
  clock: UiClock
  controller: DemoController
}

/** What a demo runs on; tests replace every part. */
export interface DemoOptions {
  openStorage?: () => Promise<Storage>
  /** The real "now", in milliseconds. */
  now?: () => number
  /** Seconds east of UTC at the instant `ms`. */
  localOffset?: (ms: number) => number
  loadCurriculum?: () => Promise<CurriculumItem[]>
}

/** Opens the demo, preparing its history first when it has none (or a stale one). */
export async function openDemo(options: DemoOptions = {}): Promise<Demo> {
  const realNow = options.now ?? (() => Date.now())
  const localOffset =
    options.localOffset ?? ((ms: number) => -new Date(ms).getTimezoneOffset() * 60)
  const curriculum = once(options.loadCurriculum ?? (() => loadBundledCurriculum()))
  const [storage, persistent] = await openStorage(options.openStorage)

  let shift = 0
  const now = () => realNow() + shift * MS_PER_DAY
  const clock: Clock = { now: () => nanosFromMillis(now()), localOffset: () => localOffset(now()) }
  const realToday = () => localDate(realNow(), localOffset(realNow()))

  const state = await readState(storage)
  if (
    state.preparedFor === null ||
    daysBetween(realToday(), state.preparedFor) >= STALE_AFTER_DAYS
  ) {
    await prepare(storage, await curriculum(), realToday(), localOffset(realNow()))
  } else {
    shift = state.dayShift
  }

  const epoch = ref(0)
  const dayShift = ref(shift)
  const busy = ref(false)
  const error = ref<string | null>(null)

  /** Runs one of the bar's actions, then remounts the app on the result. */
  async function act(action: () => Promise<number>): Promise<void> {
    if (busy.value) return
    busy.value = true
    error.value = null
    try {
      shift = await action()
      dayShift.value = shift
      epoch.value += 1
    } catch (err) {
      error.value = errorMessage(err)
    } finally {
      busy.value = false
    }
  }

  const controller: DemoController = {
    epoch,
    dayShift,
    persistent,
    busy,
    error,
    now,
    nextDay: () =>
      act(async () => {
        await storage.transaction((tx) => new Store(tx).setState(DAY_SHIFT_KEY, String(shift + 1)))
        return shift + 1
      }),
    reset: () =>
      act(async () => {
        await prepare(storage, await curriculum(), realToday(), localOffset(realNow()))
        return 0
      }),
  }
  const platform = new WebAdapter({ storage, clock, loadCurriculum: curriculum })
  return { platform, clock: { now }, controller }
}

/** IndexedDB, or memory where the browser refuses it. */
async function openStorage(open?: () => Promise<Storage>): Promise<[Storage, boolean]> {
  try {
    return [await (open ?? (() => openIndexedDbStorage(DEMO_DATABASE)))(), true]
  } catch {
    return [new MemoryStorage(), false]
  }
}

/**
 * Plays a fresh history for `today` and replaces everything stored with
 * it, the whole curriculum and the demo's keys included, in one
 * transaction.
 */
async function prepare(
  storage: Storage,
  curriculum: CurriculumItem[],
  today: IsoDate,
  offsetSeconds: number,
): Promise<void> {
  const rows = await playHistory({ today, offsetSeconds, curriculum })
  await storage.transaction(async (tx) => {
    for (const table of TABLES) tx.clear(table)
    for (const [table, entries] of rows) {
      for (const [key, row] of entries) tx.put(table, key, row)
    }
    const store = new Store(tx)
    await store.ensureCurriculumSeeded(curriculum)
    await store.setState(PREPARED_KEY, today)
  })
}

async function readState(
  storage: Storage,
): Promise<{ preparedFor: IsoDate | null; dayShift: number }> {
  return storage.transaction(async (tx) => {
    const store = new Store(tx)
    const preparedFor = await store.getState(PREPARED_KEY)
    const dayShift = Number(await store.getState(DAY_SHIFT_KEY))
    return {
      preparedFor:
        preparedFor !== null && /^\d{4}-\d{2}-\d{2}$/.test(preparedFor) ? preparedFor : null,
      dayShift: Number.isSafeInteger(dayShift) && dayShift > 0 ? dayShift : 0,
    }
  })
}

/** `load`, run once: the curriculum is fetched for the history and kept for the app. */
function once<T>(load: () => Promise<T>): () => Promise<T> {
  let loaded: Promise<T> | null = null
  return () => {
    loaded ??= load().catch((err: unknown) => {
      loaded = null
      throw err
    })
    return loaded
  }
}
