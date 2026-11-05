/**
 * The data the Automator ships under someone else's license, and the notice
 * each license asks to be shown with it (crates/vocab-core/data/README.md):
 * the bundled HSK curriculum, which Chinese captures look up for pinyin, the
 * HSK level and an example.
 *
 * The texts are the notice files next to the bundled curriculum, read at
 * build time, so what the app shows cannot drift from them: CC-CEDICT's
 * CC BY-SA 4.0 attribution, which also covers the adapted dictionary data,
 * and the MIT licenses of the two word lists. A new notice file there must
 * get an entry in {@link CREDIT_SOURCES}; a test enforces it. Learn has the
 * same module (apps/learn/src/domain/credits.ts).
 */
import type { MessageKey } from '../i18n'

export interface DataCredit {
  /** The notice file's name in crates/vocab-core/data. */
  file: string
  /** Who made the data, as its notice names them. */
  name: string
  /** What the data is, and under which license. */
  about: MessageKey
  /** The notice or license, in full. */
  text: string
}

/** Every `LICENSE-*` and `NOTICE-*` file of the bundled data, as text, by path. */
export const NOTICE_TEXTS = import.meta.glob('../../crates/vocab-core/data/{LICENSE,NOTICE}-*', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

/** One entry per notice file, in the order shown: the attribution first. */
export const CREDIT_SOURCES: Record<string, { name: string; about: MessageKey }> = {
  'NOTICE-CC-CEDICT': { name: 'CC-CEDICT (MDBG)', about: 'credits.ccCedict' },
  'LICENSE-complete-hsk-vocabulary': {
    name: 'drkameleon/complete-hsk-vocabulary',
    about: 'credits.completeHsk',
  },
  'LICENSE-hsk30': { name: 'ivankra/hsk30', about: 'credits.hsk30' },
}

/** A line that keeps its own line inside a paragraph: one copyright holder per line. */
const OWN_LINE = /^Copyright\b/

/**
 * A notice's paragraphs, with the lines that were only wrapped to fit 78
 * columns joined again (they would break raggedly in a narrower dialog),
 * except that each copyright line keeps its own line. The words stay exactly
 * as written.
 */
export function paragraphs(text: string): string[] {
  return text
    .trim()
    .split(/\n\s*\n/)
    .map((paragraph) =>
      paragraph
        .trim()
        .split(/\s*\n\s*/)
        .map((line, index) => (index === 0 ? line : (OWN_LINE.test(line) ? '\n' : ' ') + line))
        .join(''),
    )
}

/** The file name a {@link NOTICE_TEXTS} path ends in. */
export function noticeFile(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

export const DATA_CREDITS: DataCredit[] = Object.entries(CREDIT_SOURCES).map(([file, source]) => ({
  file,
  ...source,
  text: Object.entries(NOTICE_TEXTS).find(([path]) => noticeFile(path) === file)?.[1] ?? '',
}))
