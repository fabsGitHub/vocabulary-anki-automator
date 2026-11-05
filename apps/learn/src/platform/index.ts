/**
 * The running build's `PlatformAdapter` (docs/architecture.md § 10, decision
 * 10): Tauri IPC in the desktop app, IndexedDB and the TypeScript port of the
 * core in the web build.
 *
 * `__LEARN_PLATFORM__` is fixed at build time (vite.config.ts), so the branch
 * a build does not take is dead code and tree shaking drops it: the desktop
 * bundle carries no web adapter. The web adapter (`./web`, whose module
 * exports `createWebAdapter`) is reached through `import.meta.glob` rather
 * than a plain `import('./web')`: Vite still turns it into a lazily loaded
 * chunk, and this module type-checks and builds whether or not that adapter
 * is part of the checkout, since the two adapters are built independently.
 */

import { TauriAdapter } from './tauri'
import type { PlatformAdapter } from './types'

/** What `./web/index.ts` exports. */
interface WebPlatformModule {
  createWebAdapter(): Promise<PlatformAdapter>
}

const WEB_ENTRY = './web/index.ts'
const webModules = import.meta.glob<WebPlatformModule>('./web/index.ts')

export async function getPlatform(): Promise<PlatformAdapter> {
  if (__LEARN_PLATFORM__ === 'tauri') return new TauriAdapter()
  const load = webModules[WEB_ENTRY]
  if (!load) throw new Error(`This build has no web platform adapter (${WEB_ENTRY}).`)
  const { createWebAdapter } = await load()
  return createWebAdapter()
}
