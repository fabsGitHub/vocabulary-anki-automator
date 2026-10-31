/**
 * What every Learn component is handed instead of reaching for globals: the
 * platform adapter (docs/architecture.md § 10), a clock and a source of
 * exercise seeds.
 *
 * Views never import `@tauri-apps/api` or touch IndexedDB; they only talk to
 * the `PlatformAdapter` injected here, which is what lets one set of
 * components run in the desktop app and in the web build. The clock and the
 * seeds are injected for the reason the Rust core takes a `Clock`: tests pin
 * "now" (learning-step intervals and the day boundary are computed from it)
 * and the seeds that pick exercises, instead of depending on the machine.
 */

import { inject, type App, type InjectionKey } from 'vue'

import type { PlatformAdapter } from './platform/types'

/** "Now". */
export interface Clock {
  /** Milliseconds since the Unix epoch. */
  now(): number
}

/** Seeds for `nextExercise`, one per generated exercise. */
export interface SeedSource {
  /** An unsigned 32-bit integer (the core's `Mulberry32` seed). */
  next(): number
}

export interface LearnContext {
  platform: PlatformAdapter
  clock: Clock
  seeds: SeedSource
}

export const systemClock: Clock = { now: () => Date.now() }

export const randomSeeds: SeedSource = {
  next: () => crypto.getRandomValues(new Uint32Array(1))[0],
}

export const LEARN_CONTEXT_KEY: InjectionKey<LearnContext> = Symbol('learn-context')

/** A Vue plugin that gives every component of the app `context`. */
export function learnContext(context: LearnContext): { install(app: App): void } {
  return {
    install(app) {
      app.provide(LEARN_CONTEXT_KEY, context)
    },
  }
}

function useLearnContext(): LearnContext {
  const context = inject(LEARN_CONTEXT_KEY, null)
  if (!context) throw new Error('Learn components need the learnContext() plugin')
  return context
}

export function usePlatform(): PlatformAdapter {
  return useLearnContext().platform
}

export function useClock(): Clock {
  return useLearnContext().clock
}

export function useSeeds(): SeedSource {
  return useLearnContext().seeds
}
