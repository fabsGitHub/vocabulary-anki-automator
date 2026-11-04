// WCAG contrast of the site's colour tokens (src/styles.css), in light and in
// dark mode. Each pair is a colour and a background the stylesheet puts it on.
// The minimum is AA: 4.5:1 for text, 3:1 for UI components and graphics (the
// focus ring, the edge of a button against the page, the mark, icons).
// Translucent tokens (the hero glows, the text selection) are composited over
// the page background first. The .mock-* colours draw the app's own window and
// are not checked here.
//
//   npm run site:test

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { describe, test } from 'node:test'

import { SITE_DIR } from './build.mjs'

const TEXT = 4.5
const UI = 3

/** [foreground, background, minimum] by token name, without the leading `--`. */
const PAIRS = [
  ...['bg', 'bg-alt', 'surface', 'surface-2', 'glow-1', 'glow-2', 'text-selection'].map((bg) => [
    'text',
    bg,
    TEXT,
  ]),
  ...['bg', 'bg-alt', 'surface', 'surface-2'].map((bg) => ['ink', bg, TEXT]),
  ...['bg', 'bg-alt', 'surface', 'surface-2', 'accent-soft', 'glow-1', 'glow-2'].map((bg) => [
    'text-muted',
    bg,
    TEXT,
  ]),
  ...['bg', 'bg-alt', 'surface', 'surface-2'].map((bg) => ['text-subtle', bg, TEXT]),
  ...['bg', 'bg-alt', 'surface', 'surface-2', 'accent-soft'].map((bg) => ['accent-text', bg, TEXT]),
  ...['accent', 'accent-hover', 'accent-active'].map((bg) => ['on-accent', bg, TEXT]),
  ['warning-text', 'warning-bg', TEXT],
  ['selection-ink', 'selection-top', TEXT],
  ['selection-ink', 'selection-bottom', TEXT],
  ...['bg', 'bg-alt', 'surface'].map((bg) => ['accent', bg, UI]),
  ...['bg', 'bg-alt', 'surface', 'surface-2', 'accent-soft'].map((bg) => ['focus', bg, UI]),
  ...['bg', 'bg-alt'].map((bg) => ['mark', bg, UI]),
  ['caret', 'bg', UI],
  ['caret', 'selection-bottom', UI],
  ['warning-icon', 'warning-bg', UI],
]

/** The `--name: value` declarations of one CSS block. */
function declarations(block) {
  const tokens = new Map()
  for (const [, name, value] of block.matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    tokens.set(name, value.trim())
  }
  return tokens
}

/** The light tokens (the first `:root` block) and the dark ones merged over them. */
async function loadThemes() {
  const css = await readFile(path.join(SITE_DIR, 'src', 'styles.css'), 'utf8')
  const light = /^:root \{([^}]*)\}/m.exec(css)
  const dark = /@media \(prefers-color-scheme: dark\) \{\s*:root \{([^}]*)\}/.exec(css)
  assert.ok(light && dark, 'styles.css has a light and a dark :root block')
  const lightTokens = declarations(light[1])
  return { light: lightTokens, dark: new Map([...lightTokens, ...declarations(dark[1])]) }
}

/** `#rrggbb` or `rgb(r g b / a)` as [r, g, b, alpha]. */
function parseColor(value) {
  let match = /^#([0-9a-f]{6})$/i.exec(value)
  if (match) {
    const hex = match[1]
    return [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16)).concat(1)
  }
  match = /^rgb\((\d+) (\d+) (\d+) \/ ([\d.]+)\)$/.exec(value)
  if (match) return match.slice(1).map(Number)
  throw new Error(`not a #rrggbb or rgb(r g b / a) colour: ${value}`)
}

/** A token as an opaque colour, composited over the page background if translucent. */
function resolve(tokens, name) {
  assert.ok(tokens.has(name), `--${name} is defined`)
  const [r, g, b, alpha] = parseColor(tokens.get(name))
  if (alpha === 1) return [r, g, b]
  const bg = resolve(tokens, 'bg')
  return [r, g, b].map((c, i) => alpha * c + (1 - alpha) * bg[i])
}

function luminance(rgb) {
  const [r, g, b] = rgb.map((c) => {
    const s = c / 255
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

const themes = await loadThemes()

describe('colour contrast', () => {
  test('the formula matches WCAG', () => {
    assert.equal(contrast([0, 0, 0], [255, 255, 255]).toFixed(2), '21.00')
    assert.equal(contrast([255, 255, 255], [118, 118, 118]).toFixed(2), '4.54')
  })

  for (const [theme, tokens] of Object.entries(themes)) {
    test(`${theme} mode keeps AA for every text and UI colour`, () => {
      const failures = []
      for (const [fg, bg, minimum] of PAIRS) {
        const ratio = contrast(resolve(tokens, fg), resolve(tokens, bg))
        if (ratio < minimum) {
          failures.push(`--${fg} on --${bg}: ${ratio.toFixed(2)}:1, needs ${minimum}:1`)
        }
      }
      assert.deepEqual(failures, [])
    })
  }
})
