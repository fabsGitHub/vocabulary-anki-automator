// @vitest-environment node
/**
 * The release preflight (scripts/release-preflight.mjs) and the supply-chain
 * rules of both workflows (docs/signing-key-custody.md). The preflight is the
 * last gate before signing: it must refuse every updater key the updater
 * could not verify with, not just the placeholder.
 */
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

import {
  PLACEHOLDER,
  releaseProblems,
  tagProblem,
  updaterPubkeyProblem,
} from './release-preflight.mjs'

const repoFile = (path: string) => fileURLToPath(new URL(`../${path}`, import.meta.url))
const read = (path: string) => readFileSync(repoFile(path), 'utf8')
const base64 = (bytes: Uint8Array | string) => Buffer.from(bytes).toString('base64')

/** A `.key.pub` file as `tauri signer generate` writes it, base64-encoded. */
function publicKeyFile(
  algorithm = 'Ed',
  keyLength = 32,
  comment = 'minisign public key: 0123',
): string {
  const key = Buffer.concat([
    Buffer.from(algorithm, 'latin1'),
    Buffer.alloc(8, 7),
    Buffer.alloc(keyLength, 9),
  ])
  return base64(`untrusted comment: ${comment}\n${base64(key)}\n`)
}

describe('updater public key', () => {
  it('accepts a minisign public key', () => {
    expect(updaterPubkeyProblem(publicKeyFile())).toBeNull()
    expect(updaterPubkeyProblem(publicKeyFile('ED'))).toBeNull()
    // Windows line endings in the decoded file.
    const crlf = base64(
      `untrusted comment: minisign public key: 0123\r\n${base64(Buffer.concat([Buffer.from('Ed'), Buffer.alloc(40, 1)]))}\r\n`,
    )
    expect(updaterPubkeyProblem(crlf)).toBeNull()
  })

  it.each([
    ['the placeholder', PLACEHOLDER, /placeholder/],
    ['an empty key', '', /empty/],
    ['whitespace', '   ', /empty/],
    ['a missing key', undefined, /empty/],
    ['text that is not base64', 'not a key!', /not base64/],
    ['base64 of one line', base64('untrusted comment: minisign public key'), /not a minisign/],
    ['a key of the wrong length', publicKeyFile('Ed', 31), /not a minisign/],
    ['an unknown algorithm', publicKeyFile('XX'), /not a minisign/],
    [
      'the secret key',
      base64(`untrusted comment: rsign encrypted secret key\n${base64(Buffer.alloc(158, 3))}\n`),
      /SECRET key/,
    ],
  ])('refuses %s', (_, pubkey, problem) => {
    expect(updaterPubkeyProblem(pubkey)).toMatch(problem)
  })
})

describe('tag', () => {
  it('must be v<version>', () => {
    expect(tagProblem('v0.1.0', '0.1.0')).toBeNull()
    expect(tagProblem('v0.1.1', '0.1.0')).toMatch(/does not match/)
    expect(tagProblem('0.1.0', '0.1.0')).toMatch(/not v<version>/)
    expect(tagProblem(undefined, '0.1.0')).toMatch(/not v<version>/)
  })
})

describe('the committed configuration', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'))

  it('is refused until the operator commits the real public key', () => {
    // The placeholder is still in place (IMPLEMENTATION_PLAN.md N4, operator step).
    expect(releaseProblems(config, `v${config.version}`)).toEqual([
      expect.stringMatching(/placeholder/),
    ])
    const withKey = { ...config, plugins: { updater: { pubkey: publicKeyFile() } } }
    expect(releaseProblems(withKey, `v${config.version}`)).toEqual([])
  })

  it('makes the script exit non-zero with a workflow error', () => {
    let failure: { status: number; stdout: string } | null = null
    try {
      execFileSync(process.execPath, [repoFile('scripts/release-preflight.mjs'), 'v9.9.9'], {
        encoding: 'utf8',
      })
    } catch (err) {
      failure = err as { status: number; stdout: string }
    }
    expect(failure?.status).toBe(1)
    expect(failure?.stdout).toMatch(/^::error::plugins\.updater\.pubkey is still the placeholder/m)
    expect(failure?.stdout).toMatch(/^::error::tag v9\.9\.9 does not match/m)
  })
})

describe('workflows', () => {
  const release = read('.github/workflows/release.yml')
  const ci = read('.github/workflows/ci.yml')

  it('run the preflight before the build job may start', () => {
    expect(release).toMatch(/run: node scripts\/release-preflight\.mjs "\$GITHUB_REF_NAME"/)
    expect(release).toMatch(/^ {2}build:\n {4}needs: preflight$/m)
  })

  it('pin every action to a full commit SHA', () => {
    for (const workflow of [release, ci]) {
      const uses = [...workflow.matchAll(/^\s*(?:-\s+)?uses:\s*(\S+)/gm)].map((m) => m[1])
      expect(uses.length).toBeGreaterThan(0)
      for (const action of uses) expect(action).toMatch(/^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/)
    }
  })

  it('never store the token in the checkout', () => {
    for (const workflow of [release, ci]) {
      const checkouts = [...workflow.matchAll(/uses: actions\/checkout@\S+[^\n]*\n(.*)\n(.*)/g)]
      expect(checkouts.length).toBeGreaterThan(0)
      for (const [, withLine, option] of checkouts) {
        expect(withLine.trim()).toBe('with:')
        expect(option.trim()).toBe('persist-credentials: false')
      }
    }
  })

  it('grant write access to the release job only', () => {
    expect(release).toMatch(/^permissions:\n {2}contents: read$/m)
    expect(ci).toMatch(/^permissions:\n {2}contents: read$/m)
    const writes = [...release.matchAll(/contents: write/g)]
    expect(writes).toHaveLength(1)
    expect(release).toMatch(
      /^ {2}build:\n {4}needs: preflight\n {4}permissions:\n {6}contents: write$/m,
    )
  })

  it('reference secrets by name only, and Apple ones on macOS only', () => {
    const references = [...release.matchAll(/\$\{\{([^}]*secrets\.[^}]*)\}\}/g)].map((m) =>
      m[1].trim(),
    )
    expect(references.length).toBeGreaterThan(0)
    for (const reference of references) {
      const [, name] = /secrets\.([A-Z_]+)/.exec(reference) ?? []
      if (name?.startsWith('APPLE_')) {
        expect(reference).toBe(`runner.os == 'macOS' && secrets.${name} || ''`)
      } else {
        expect(reference).toBe(`secrets.${name}`)
      }
    }
    // Nothing prints a secret or turns tracing on.
    expect(release).not.toMatch(/echo[^\n]*(secrets\.|\$\{?(TAURI_SIGNING|APPLE_|WINDOWS_CERT))/i)
    expect(release).not.toMatch(/set -x/)
    expect(ci).not.toMatch(/secrets\./)
  })
})
