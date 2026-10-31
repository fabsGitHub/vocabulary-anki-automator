import { describe, expect, it } from 'vitest'

import packageJson from '../../../../package.json'
import justfile from '../../../../justfile?raw'
import tauriConf from '../../src-tauri/tauri.conf.json'

// Which PlatformAdapter a Learn bundle gets is decided by Vite's mode
// (apps/learn/vite.config.ts): `--mode web` is the browser version over
// IndexedDB, any other mode the desktop one, whose every call is a Tauri
// `invoke` and fails in a plain browser. So every way of starting or
// building Learn has to pass the mode of the platform it is for.

const scripts: Record<string, string> = packageJson.scripts

function webMode(script: string | undefined): boolean {
  return / --mode web(\s|$)/.test(script ?? '')
}

/** The commands of the just recipe `name`. */
function recipe(name: string): string[] {
  const lines = justfile.split('\n')
  const start = lines.findIndex((line) => line.startsWith(`${name}:`))
  if (start === -1) return []
  const body: string[] = []
  for (const line of lines.slice(start + 1)) {
    if (!/^\s+\S/.test(line)) break
    body.push(line.trim())
  }
  return body
}

describe('Learn builds and dev servers', () => {
  it('give the Tauri app the desktop frontend', () => {
    expect(tauriConf.build.beforeDevCommand.script).toBe('npm run learn:dev')
    expect(tauriConf.build.beforeBuildCommand.script).toBe('npm run learn:build')
    expect(webMode(scripts['learn:dev'])).toBe(false)
    expect(webMode(scripts['learn:build'])).toBe(false)
    expect(recipe('learn-dev')).toEqual(['npm run learn:tauri dev'])
  })

  it('serve and build the browser version in web mode', () => {
    expect(webMode(scripts['learn:build:web'])).toBe(true)
    expect(webMode(scripts['learn:dev:web'])).toBe(true)
    expect(recipe('learn-web')).toEqual(['npm run learn:dev:web'])
  })
})
