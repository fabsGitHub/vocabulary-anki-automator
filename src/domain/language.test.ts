import { describe, expect, it } from 'vitest'

import { isChinese, isSupportedLang, normalizeLang } from './language'

describe('normalizeLang (mirrors vocab_core::language)', () => {
  it.each([
    ['en', 'en'],
    ['EN', 'en'],
    [' En ', 'en'],
    ['DE', 'de'],
    ['ES', 'es'],
    ['It', 'it'],
    ['zh', 'zh-CN'],
    ['ZH', 'zh-CN'],
    ['zh-cn', 'zh-CN'],
    ['ZH-CN', 'zh-CN'],
    ['zh_CN', 'zh-CN'],
    ['zh-Hans', 'zh-CN'],
    ['zh_Hans', 'zh-CN'],
    ['zh-Hans-CN', 'zh-CN'],
    ['\tzh\n', 'zh-CN'],
    ['PT', 'pt'],
    [' ja ', 'ja'],
    ['en-US', 'en-us'],
    ['en_US', 'en_us'],
  ])('%j -> %j', (input, expected) => {
    expect(normalizeLang(input)).toBe(expected)
  })
})

describe('isSupportedLang', () => {
  it('accepts exactly the canonical codes, including zh-CN', () => {
    for (const code of ['en', 'de', 'fr', 'es', 'it', 'zh-CN'])
      expect(isSupportedLang(code)).toBe(true)
    for (const code of ['EN', 'zh', 'zh-cn', 'pt', '']) expect(isSupportedLang(code)).toBe(false)
  })
})

describe('isChinese', () => {
  it('recognizes every accepted spelling of Simplified Chinese', () => {
    expect(isChinese('zh_CN')).toBe(true)
    expect(isChinese('zh-Hans')).toBe(true)
    expect(isChinese('ja')).toBe(false)
  })
})
