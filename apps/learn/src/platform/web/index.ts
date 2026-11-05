/**
 * Learn's web platform (decision 10, docs/architecture.md § 12 item 7):
 * the {@link PlatformAdapter} of the standalone browser build.
 *
 * A browser cannot run the Rust core (compiling it to WebAssembly needed a
 * toolchain change that was not approved), so this directory is a
 * TypeScript port of the parts of `vocab-core` Learn uses, module for module
 * and function for function, over IndexedDB instead of SQLite:
 *
 * | module                                | ports                                         |
 * |---------------------------------------|-----------------------------------------------|
 * | `rng`                                 | `rng::Mulberry32`                             |
 * | `fsrs`                                | rs-fsrs 1.2.1, basic and long-term schedulers |
 * | `scheduler`                           | `srs::scheduler` (`SrsScheduler`, rows)       |
 * | `queue`, `daily`                      | `srs::queue`, `srs::daily`                    |
 * | `pool`, `segment`, `exercises`        | `quiz`: the learned pool, tiles, exercises    |
 * | `gamification`, `lessons`             | `gamification`, `lessons`                     |
 * | `store`, `curriculum`                 | `store`: tables, rules, fingerprint           |
 * | `language`, `text`, `time`, `model`   | what those build on; `errors` their errors    |
 * | `session`, `progress`, `settings`     | vocab-learn's modules of the same names       |
 * | `adapter`                             | vocab-learn's commands                        |
 * | `storage`, `idb`                      | transactions over IndexedDB, or memory        |
 *
 * The port is pinned to the Rust core by golden vectors:
 * `crates/vocab-core/examples/golden_vectors.rs` runs the real Rust
 * implementations on fixed inputs and writes `__fixtures__/golden.json`,
 * and `golden.test.ts` replays every input here and expects the same
 * outputs.
 */

import type { PlatformAdapter } from '../types'
import { WebAdapter, type WebAdapterOptions } from './adapter'
import { openIndexedDbStorage } from './idb'

/**
 * The web build's adapter: IndexedDB, the system clock and the bundled
 * curriculum unless `options` say otherwise. It resolves even when
 * IndexedDB cannot be opened; every command then rejects with the reason.
 */
export async function createWebAdapter(
  options: Partial<WebAdapterOptions> = {},
): Promise<PlatformAdapter> {
  return new WebAdapter({ ...options, storage: options.storage ?? openIndexedDbStorage() })
}

export { WebAdapter, type WebAdapterOptions } from './adapter'
export { MemoryStorage, type Storage } from './storage'
export { FixedClock, systemClock, type Clock } from './time'
