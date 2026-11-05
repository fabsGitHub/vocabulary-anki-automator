/**
 * Loading the bundled curriculum in the browser (`./curriculum.ts`): the
 * asset is fetched, parsed with serde's rules for `CurriculumItem` and put
 * in `curriculum_order`. The fingerprint itself is pinned by golden.test.ts.
 */

import { describe, expect, it } from 'vitest'

import { loadBundledCurriculum, parseCurriculum } from './curriculum'

const ROWS = [
  { hanzi: '你', hsk_level: 1, curriculum_order: 2, pinyin: 'nǐ', meaning_en: 'you' },
  { hanzi: '我', hsk_level: 1, curriculum_order: 1, pinyin: 'wǒ', frequency: 3 },
]

describe('the bundled curriculum', () => {
  it('is fetched from its asset URL once and comes back in curriculum order', async () => {
    const asked: string[] = []
    const items = await loadBundledCurriculum(async (url) => {
      asked.push(url)
      return new Response(JSON.stringify(ROWS))
    })
    expect(asked).toHaveLength(1)
    expect(asked[0]).toMatch(/hsk_curriculum\.json/)
    expect(items.map((item) => item.hanzi)).toEqual(['我', '你'])
    // Fields the file leaves out are serde's defaults.
    expect(items[0]).toMatchObject({ frequency: 3, meaning_en: null, traditional: null })
    expect(items[1]).toMatchObject({ frequency: null, meaning_en: 'you' })
  })

  it('reports a download that failed', async () => {
    await expect(
      loadBundledCurriculum(async () => new Response('missing', { status: 404 })),
    ).rejects.toThrow('failed with HTTP 404')
  })

  it('refuses rows the desktop parser would refuse instead of seeding empty words', () => {
    expect(() => parseCurriculum({ rows: ROWS })).toThrow('not a JSON array')
    expect(() => parseCurriculum([ROWS[0], 'row'])).toThrow('curriculum row 1 is not an object')
    expect(() => parseCurriculum([{ ...ROWS[0], hanzi: undefined }])).toThrow(
      'curriculum row 0 has no string `hanzi`',
    )
    expect(() => parseCurriculum([{ ...ROWS[1], curriculum_order: '1' }])).toThrow(
      'curriculum row 0 has no number `curriculum_order`',
    )
  })
})
