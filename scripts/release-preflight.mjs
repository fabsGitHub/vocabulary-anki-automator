#!/usr/bin/env node
// Release preflight (Milestones N4/N5), run by .github/workflows/release.yml
// before anything is built or signed: `node scripts/release-preflight.mjs <tag>`.
//
// It refuses a release whose updater could never verify an update, which
// would strand every installed copy on that version, and a tag that is not
// the app's version. The key check mirrors what tauri-plugin-updater does
// with `plugins.updater.pubkey` (verify_signature -> minisign-verify
// PublicKey::decode): base64 of the two-line `.key.pub` file that
// `tauri signer generate` writes, whose second line is the base64 of a
// 42-byte key ("Ed" or "ED", 8-byte key id, 32-byte Ed25519 key). Only the
// public key is ever read; no secret is needed or touched here.
// Tested by scripts/release-preflight.test.ts.

import { readFileSync, realpathSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

export const PLACEHOLDER = 'KEYS_PLACEHOLDER_REPLACE_AFTER_GENERATION'

const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/

function decodeBase64(text) {
  const compact = text.trim()
  if (compact.length === 0 || compact.length % 4 !== 0 || !BASE64.test(compact)) return null
  return Buffer.from(compact, 'base64')
}

/** Why `pubkey` cannot verify updates, or `null` when it is a minisign public key. */
export function updaterPubkeyProblem(pubkey) {
  if (typeof pubkey !== 'string' || pubkey.trim() === '') {
    return 'plugins.updater.pubkey is empty.'
  }
  if (pubkey.includes(PLACEHOLDER)) {
    return (
      'plugins.updater.pubkey is still the placeholder. Run `tauri signer generate` ' +
      '(docs/signing-key-custody.md) and commit the public key first.'
    )
  }
  const file = decodeBase64(pubkey)
  if (!file) return 'plugins.updater.pubkey is not base64 (paste the content of the .key.pub file).'
  const text = file.toString('utf8')
  if (/secret key/i.test(text)) {
    return (
      'plugins.updater.pubkey holds a SECRET key. Treat it as leaked: generate a new ' +
      'key pair, store only the private key as the TAURI_SIGNING_PRIVATE_KEY secret, ' +
      'and commit the new public key.'
    )
  }
  const lines = text.split('\n').map((line) => line.replace(/\r$/, ''))
  const key = lines.length >= 2 ? decodeBase64(lines[1]) : null
  const algorithm = key ? key.subarray(0, 2).toString('latin1') : ''
  if (!key || key.length !== 42 || (algorithm !== 'Ed' && algorithm !== 'ED')) {
    return (
      'plugins.updater.pubkey is not a minisign public key (expected the base64 of the ' +
      '.key.pub file that `tauri signer generate` wrote).'
    )
  }
  return null
}

/** Why the pushed tag does not release `version`, or `null`. */
export function tagProblem(tag, version) {
  if (typeof tag !== 'string' || !tag.startsWith('v')) {
    return `the tag ${JSON.stringify(tag)} is not v<version>.`
  }
  if (tag.slice(1) !== version) {
    return `tag ${tag} does not match src-tauri/tauri.conf.json version ${version}.`
  }
  return null
}

/** Every problem with releasing `config` (tauri.conf.json, parsed) as `tag`. */
export function releaseProblems(config, tag) {
  return [
    updaterPubkeyProblem(config?.plugins?.updater?.pubkey),
    tagProblem(tag, config?.version),
  ].filter((problem) => problem !== null)
}

function main() {
  const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME
  const configPath = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url))
  const problems = releaseProblems(JSON.parse(readFileSync(configPath, 'utf8')), tag)
  for (const problem of problems) console.log(`::error::${problem}`)
  if (problems.length > 0) process.exit(1)
  console.log(`release preflight passed for ${tag}`)
}

if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
  main()
}
