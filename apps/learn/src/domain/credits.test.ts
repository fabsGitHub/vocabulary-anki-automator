import { describe, expect, it } from 'vitest'

import { CREDIT_SOURCES, DATA_CREDITS, NOTICE_TEXTS, noticeFile, paragraphs } from './credits'

const text = (file: string) => DATA_CREDITS.find((credit) => credit.file === file)?.text ?? ''

describe('the data credits', () => {
  it('cover every notice file of the bundled data, and nothing else', () => {
    const files = Object.keys(NOTICE_TEXTS).map(noticeFile).sort()
    expect(files).toEqual(['LICENSE-complete-hsk-vocabulary', 'LICENSE-hsk30', 'NOTICE-CC-CEDICT'])
    expect(Object.keys(CREDIT_SOURCES).sort()).toEqual(files)
  })

  it('carry CC-CEDICT’s attribution first, with its license and source', () => {
    expect(DATA_CREDITS[0].file).toBe('NOTICE-CC-CEDICT')
    expect(text('NOTICE-CC-CEDICT')).toContain('CC-CEDICT')
    expect(text('NOTICE-CC-CEDICT')).toContain('https://creativecommons.org/licenses/by-sa/4.0/')
    expect(text('NOTICE-CC-CEDICT')).toContain(
      'https://www.mdbg.net/chinese/dictionary?page=cc-cedict',
    )
  })

  it('carry both MIT licenses in full, copyright line included', () => {
    for (const file of ['LICENSE-complete-hsk-vocabulary', 'LICENSE-hsk30']) {
      expect(text(file)).toMatch(/^MIT License\s+Copyright \(c\) \d{4} \S/)
      expect(text(file)).toContain('Permission is hereby granted, free of charge')
      expect(text(file)).toContain('THE SOFTWARE IS PROVIDED "AS IS"')
    }
  })

  it('reflow each notice into its paragraphs, keeping every word and each copyright line', () => {
    expect(
      paragraphs(
        'FITNESS FOR A PARTICULAR PURPOSE AND\nNONINFRINGEMENT.\n\n  Copyright (c) 2023 A\n  Copyright (c) 2021 B\n',
      ),
    ).toEqual([
      'FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.',
      'Copyright (c) 2023 A\nCopyright (c) 2021 B',
    ])
    for (const credit of DATA_CREDITS) {
      const words = (text: string) => text.split(/\s+/).filter(Boolean)
      expect(words(paragraphs(credit.text).join(' ')), credit.file).toEqual(words(credit.text))
    }
  })
})
