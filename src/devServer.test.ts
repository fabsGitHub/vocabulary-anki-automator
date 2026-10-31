/**
 * @vitest-environment node
 *
 * The Automator's dev server (`npm run dev`, the `beforeDevCommand` of
 * `tauri dev`) runs Vite from the repo root, so it watches everything below
 * it that it is not told to ignore. Cargo's workspace target dir is there too
 * (Milestone B moved it out of src-tauri/), and `tauri dev` rebuilds into it
 * while Vite runs: watched, it costs a watch per directory (thousands of
 * them) and floods the watcher on every rebuild. The config is Node code
 * (Vite and esbuild do not load under jsdom), hence the Node environment.
 */

import { createFilter, type UserConfig, type UserConfigFnPromise } from 'vite'
import { describe, expect, it } from 'vitest'

// Through Vite rather than a typed import: vite.config.ts belongs to the
// tsconfig.node.json project, which the frontend's type check does not build.
const viteConfig = import.meta.glob<UserConfigFnPromise>('../vite.config.ts', {
  eager: true,
  import: 'default',
})['../vite.config.ts']

/** Whether the dev server watches `path`, matched as chokidar does (picomatch globs). */
async function watchedBy(config: UserConfig): Promise<(path: string) => boolean> {
  const ignored = config.server?.watch?.ignored ?? []
  const globs = (Array.isArray(ignored) ? ignored : [ignored]).filter(
    (matcher): matcher is string => typeof matcher === 'string',
  )
  return createFilter(null, globs, { resolve: false })
}

describe("the Automator's dev server", () => {
  it('watches the frontend but not the Rust side or Cargo output', async () => {
    const watched = await watchedBy(await viteConfig({ command: 'serve', mode: 'development' }))
    expect(watched('/repo/src/App.vue')).toBe(true)
    expect(watched('/repo/src-tauri/src/lib.rs')).toBe(false)
    expect(watched('/repo/target/debug/build/vocab-automator-1/out/codegen.rs')).toBe(false)
    expect(watched('/repo/target/release/vocab-automator')).toBe(false)
  })
})
