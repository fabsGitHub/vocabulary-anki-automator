import { describe, expect, it } from 'vitest'

import architecture from '../../docs/architecture.md?raw'
import {
  ANKICONNECT_COMMIT,
  ANKICONNECT_FILES,
  ANKI_INSTALLERS,
  ANKI_PINNED_VERSION,
} from './supplyChain'

/** The consent dialog must state the very pins the contract (§ 8.4) records. */
describe('consent dialog pins', () => {
  it('name the pinned AnkiConnect commit', () => {
    expect(architecture).toContain(`commit\n  \`${ANKICONNECT_COMMIT}\``)
  })

  it('list every AnkiConnect file with its contract checksum', () => {
    const rows = [...architecture.matchAll(/\| `([\w.]+\.py)`\s+\| `([0-9a-f]{64})` \|/g)]
    const documented = rows.map(([, name, sha256]) => ({ name, sha256 }))
    expect(documented).toHaveLength(4)
    expect([...ANKICONNECT_FILES]).toEqual(documented)
  })

  it('list every Anki installer with its contract checksum', () => {
    const rows = [...architecture.matchAll(/\| `(anki-[\w.-]+)`\s+\| `([0-9a-f]{64})` \|/g)]
    const documented = rows.map(([, name, sha256]) => ({ name, sha256 }))
    expect(documented).toHaveLength(3)
    expect(ANKI_INSTALLERS.map(({ name, sha256 }) => ({ name, sha256 }))).toEqual(documented)
    expect(architecture).toContain(`Anki is pinned to ${ANKI_PINNED_VERSION}`)
  })
})
