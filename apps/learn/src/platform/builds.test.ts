/**
 * @vitest-environment node
 *
 * Every way of serving or building Learn gets the platform it is for
 * (decision 10, docs/architecture.md § 10). `vite.config.ts` fixes
 * `__LEARN_PLATFORM__` from Vite's mode and selects the web adapter only in
 * `--mode web`, so a browser script that forgets the flag serves the Tauri
 * adapter, whose every call fails outside the desktop app.
 *
 * The desktop side is what Tauri itself runs (`tauri.conf.json`'s
 * `beforeDevCommand` and `beforeBuildCommand`); the browser side is what
 * `just learn-web` and CI run. The config is Node code (Vite and esbuild do
 * not load under jsdom), hence the Node environment.
 */

import { build, type Rollup, type UserConfig } from 'vite'
import { describe, expect, it } from 'vitest'

import justfile from '../../../../justfile?raw'
import packageJson from '../../../../package.json'
import tauriConf from '../../src-tauri/tauri.conf.json'

interface ViteEnv {
  command: 'build' | 'serve'
  mode: string
}

interface LearnViteConfig extends UserConfig {
  define: { __LEARN_PLATFORM__: string }
  build: { outDir: string }
}

// Loaded through Vite instead of a typed import: the config is Node code,
// which Learn's browser-only type check does not cover.
const configModules = import.meta.glob<(env: ViteEnv) => LearnViteConfig>('../../vite.config.ts', {
  eager: true,
  import: 'default',
})
const learnViteConfig = configModules['../../vite.config.ts']

const scripts: Record<string, string> = packageJson.scripts

/** How `npm run <script>` (from the repo root) runs Vite: command and mode. */
function viteEnvOf(npmRun: string): ViteEnv {
  const name = /^npm run (\S+)$/.exec(npmRun.trim())?.[1] ?? ''
  const script = scripts[name] ?? ''
  const vite = script
    .split('&&')
    .map((part) => part.trim())
    .find((part) => /^vite\b/.test(part))
  expect(vite, `\`${npmRun}\` runs Vite`).toBeDefined()
  expect(vite).toContain('--config apps/learn/vite.config.ts')
  const command = /^vite build\b/.test(vite ?? '') ? 'build' : 'serve'
  const mode = /--mode[= ](\S+)/.exec(vite ?? '')?.[1]
  return { command, mode: mode ?? (command === 'build' ? 'production' : 'development') }
}

function platformOf(npmRun: string): string {
  return JSON.parse(learnViteConfig(viteEnvOf(npmRun)).define.__LEARN_PLATFORM__)
}

/** The first `npm run` line of a justfile recipe (whose lines a Windows checkout ends in CRLF). */
function recipeNpmRun(recipe: string): string {
  const header = `^${recipe}:[^\\r\\n]*\\r?\\n`
  const body = new RegExp(`${header}((?:[ \\t]+\\S[^\\r\\n]*(?:\\r?\\n|$))+)`, 'm').exec(justfile)
  return /npm run \S+/.exec(body?.[1] ?? '')?.[0] ?? `(no npm run in just ${recipe})`
}

/** The files `npm run <script>` builds, in memory: Vite with Learn's config, nothing written. */
async function builtFiles(npmRun: string): Promise<string[]> {
  const env = viteEnvOf(npmRun)
  const config = learnViteConfig(env)
  const result = await build({
    ...config,
    configFile: false,
    mode: env.mode,
    logLevel: 'silent',
    build: { ...config.build, write: false },
  })
  const outputs = (Array.isArray(result) ? result : [result]) as Rollup.RollupOutput[]
  return outputs.flatMap((output) => output.output.map((file) => file.fileName))
}

describe("Learn's builds", () => {
  it('give the desktop app the Tauri platform, written where Tauri embeds it', () => {
    const { beforeDevCommand, beforeBuildCommand, frontendDist } = tauriConf.build
    // The hooks run where package.json is: three levels above apps/learn/src-tauri.
    expect(beforeDevCommand.cwd).toBe('../../..')
    expect(beforeBuildCommand.cwd).toBe('../../..')
    expect(platformOf(beforeDevCommand.script)).toBe('tauri')
    expect(platformOf(beforeBuildCommand.script)).toBe('tauri')
    const outDir = learnViteConfig(viteEnvOf(beforeBuildCommand.script)).build.outDir
    expect(`../${outDir}`).toBe(frontendDist)
  })

  it('give the browser the web platform, in its own output directory', () => {
    expect(platformOf(recipeNpmRun('learn-web'))).toBe('web')
    expect(platformOf('npm run learn:build:web')).toBe('web')
    expect(learnViteConfig(viteEnvOf('npm run learn:build:web')).build.outDir).toBe('dist-web')
  })

  it('ship the curriculum JSON only in the web build', async () => {
    // The desktop app compiles it into its binary; only the web adapter
    // fetches it, and the desktop build drops that adapter's code.
    const hasCurriculum = (files: string[]) => files.some((file) => file.includes('hsk_curriculum'))
    expect(hasCurriculum(await builtFiles(tauriConf.build.beforeBuildCommand.script))).toBe(false)
    expect(hasCurriculum(await builtFiles('npm run learn:build:web'))).toBe(true)
  }, 60_000)
})
