/**
 * The bundled HSK 3.0 curriculum in the web build (decision 9), and the
 * fingerprint that decides when to seed it.
 *
 * The desktop apps compile `crates/vocab-core/data/hsk_curriculum.json` into
 * their binaries. The web build ships the same file as a static asset
 * instead and fetches it only when the queue or a new word first needs it:
 * at some 2 MB it would otherwise weigh down the first screen, which is the
 * daily prompt and needs no curriculum at all.
 *
 * Seeding writes the 11,000 rows only when their fingerprint differs from
 * the one recorded in `app_state`, as `Store::ensure_curriculum_seeded`
 * does. The fingerprint is the store's FNV-1a 64 over every field, ported
 * byte for byte, so the web build and the desktop file agree on what
 * "the same curriculum" means; the golden vectors pin it, the bundled
 * file's included.
 */

import curriculumUrl from '../../../../../crates/vocab-core/data/hsk_curriculum.json?url'
import { type CurriculumItem, curriculumItemFromJson } from './model'
import { compareStrings } from './text'

/** `store::CURRICULUM_FINGERPRINT_KEY`. */
export const CURRICULUM_FINGERPRINT_KEY = 'curriculum_fingerprint'

/**
 * `curriculum::bundled_curriculum`: the bundled rows, ordered by
 * `curriculum_order`. `fetchUrl` stands in for `fetch` in tests.
 */
export async function loadBundledCurriculum(
  fetchUrl: (url: string) => Promise<Response> = (url) => fetch(url),
): Promise<CurriculumItem[]> {
  const response = await fetchUrl(curriculumUrl)
  if (!response.ok) throw new Error(`loading ${curriculumUrl} failed with HTTP ${response.status}`)
  return parseCurriculum(await response.json())
}

/** The curriculum JSON's rows, ordered by `curriculum_order`. */
export function parseCurriculum(value: unknown): CurriculumItem[] {
  if (!Array.isArray(value)) throw new Error('the curriculum is not a JSON array')
  const items = value.map((item: unknown) => {
    if (item === null || typeof item !== 'object')
      throw new Error('a curriculum row is not an object')
    return curriculumItemFromJson(item as Partial<CurriculumItem>)
  })
  return items.sort((a, b) => a.curriculum_order - b.curriculum_order)
}

/**
 * `store::curriculum_fingerprint`: FNV-1a 64 over every field of every item,
 * visited in `curriculum_order` (ties by `hanzi`), as 16 lowercase hex
 * digits. Strings are prefixed with their UTF-8 length and `null` differs
 * from `""`, so moving text between fields changes it too.
 */
export function curriculumFingerprint(items: readonly CurriculumItem[]): string {
  const ordered = [...items].sort(
    (a, b) => a.curriculum_order - b.curriculum_order || compareStrings(a.hanzi, b.hanzi),
  )
  const hash = new Fnv1a64()
  for (const item of ordered) hashItem(hash, item)
  return hash.finish()
}

function hashItem(hash: Fnv1a64, item: CurriculumItem): void {
  hash.writeStr(item.hanzi)
  hash.writeOptStr(item.traditional)
  hash.write([item.hsk_level & 0xff])
  hash.writeOptI64(item.frequency)
  hash.writeI64(item.curriculum_order)
  hash.writeStr(item.pinyin)
  for (const text of [
    item.pos,
    item.meaning_en,
    item.meaning_de,
    item.cloze_sentence,
    item.example_sentence_source,
    item.example_sentence_pinyin,
    item.example_sentence_target,
    item.vocab_audio_path,
    item.sentence_audio_path,
    item.image_path,
  ]) {
    hash.writeOptStr(text)
  }
}

/**
 * 64-bit FNV-1a in four 16-bit limbs, so that every intermediate product
 * stays an exact JavaScript number. The prime is 2^40 + 0x1b3, so a
 * multiplication is `h * 0x1b3` plus `h` shifted up by 40 bits.
 */
export class Fnv1a64 {
  private limbs = [0x2325, 0x8422, 0x9ce4, 0xcbf2]

  write(bytes: Iterable<number>): void {
    let [h0, h1, h2, h3] = this.limbs
    for (const byte of bytes) {
      h0 ^= byte
      const t0 = h0 * 0x1b3
      const t1 = h1 * 0x1b3 + (t0 >>> 16)
      const t2 = h2 * 0x1b3 + (t1 >>> 16) + (h0 << 8)
      const t3 = h3 * 0x1b3 + (t2 >>> 16) + (h1 << 8)
      h0 = t0 & 0xffff
      h1 = t1 & 0xffff
      h2 = t2 & 0xffff
      h3 = t3 & 0xffff
    }
    this.limbs = [h0, h1, h2, h3]
  }

  /** `write_i64`: eight little-endian bytes of the two's complement. */
  writeI64(value: number): void {
    let bits = BigInt.asUintN(64, BigInt(value))
    const bytes: number[] = []
    for (let index = 0; index < 8; index += 1) {
      bytes.push(Number(bits & 0xffn))
      bits >>= 8n
    }
    this.write(bytes)
  }

  /** `write_str`: the UTF-8 length as a little-endian `u64`, then the bytes. */
  writeStr(text: string): void {
    const bytes = utf8(text)
    this.writeI64(bytes.length)
    this.write(bytes)
  }

  writeOptStr(text: string | null): void {
    if (text === null) {
      this.write([0])
    } else {
      this.write([1])
      this.writeStr(text)
    }
  }

  writeOptI64(value: number | null): void {
    if (value === null) {
      this.write([0])
    } else {
      this.write([1])
      this.writeI64(value)
    }
  }

  /** The hash as 16 lowercase hex digits. */
  finish(): string {
    return [...this.limbs]
      .reverse()
      .map((limb) => limb.toString(16).padStart(4, '0'))
      .join('')
  }
}

/** The UTF-8 bytes of `text` (a lone surrogate as U+FFFD, as `TextEncoder` writes it). */
export function utf8(text: string): number[] {
  const bytes: number[] = []
  for (const c of text) {
    let point = c.codePointAt(0) ?? 0
    if (point >= 0xd800 && point <= 0xdfff) point = 0xfffd
    if (point < 0x80) {
      bytes.push(point)
    } else if (point < 0x800) {
      bytes.push(0xc0 | (point >> 6), 0x80 | (point & 0x3f))
    } else if (point < 0x10000) {
      bytes.push(0xe0 | (point >> 12), 0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f))
    } else {
      bytes.push(
        0xf0 | (point >> 18),
        0x80 | ((point >> 12) & 0x3f),
        0x80 | ((point >> 6) & 0x3f),
        0x80 | (point & 0x3f),
      )
    }
  }
  return bytes
}
