#!/usr/bin/env node
// Draws the social preview images (site/src/assets/og-en.png, og-de.png) and
// the touch icon (apple-touch-icon.png). The previews use the site's own
// artwork: the mark (src/partials/mark.svg), the headline with its selected
// word, and a drawing of the approval window with the app's real labels, all
// taken from the page copy in site/src/i18n. The touch icon is the app icon
// (branding/icon.svg) cropped to its body and filled to the corners, because
// iOS rounds the corners itself and would show transparent ones as black.
//
// The build does not run this: the PNGs are committed. Run it again after the
// copy, the mark or the icon changes. It needs `rsvg-convert` (librsvg, e.g.
// `brew install librsvg`) and works offline.
//
//   node site/og/render.mjs [--keep-svg <dir>]

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { inflateSync } from 'node:zlib'

const SITE_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const REPO_ROOT = path.resolve(SITE_DIR, '..')
const ASSETS = path.join(SITE_DIR, 'src', 'assets')

const FONT = "'Helvetica Neue', Helvetica, Arial, sans-serif"
/** The icon's palette (branding/README.md) and the page's tokens (src/styles.css). */
const CORAL = '#ff9a5c'
const RASPBERRY = '#e2306b'
const INK = '#2c0f40'
const SELECTION = ['#ae98ff', '#9478fa']
const ACCENT_TEXT = '#bf1b51'
const ACCENT_SOFT = '#fdeef3'
const ACCENT_BORDER = '#f5c2d3'
const BORDER_STRONG = '#d6cede'

const esc = (value) =>
  String(value).replace(
    /[&<>"]/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c],
  )

/** Rough advance width of Helvetica-like text, enough to size badges. */
function textWidth(text, size, weight = 400) {
  const factor = weight >= 600 ? 0.58 : 0.54
  return [...text].length * size * factor
}

/** Greedy word wrap to at most `max` characters per line. */
function wrap(text, max) {
  const lines = []
  let line = ''
  for (const word of text.split(' ')) {
    if (line && (line + ' ' + word).length > max) {
      lines.push(line)
      line = word
    } else {
      line = line ? `${line} ${word}` : word
    }
  }
  if (line) lines.push(line)
  return lines
}

/** The mark: the path of src/partials/mark.svg (64 grid) in the icon's body gradient. */
function mark(glyph, x, y, size) {
  return `<g transform="translate(${x} ${y}) scale(${size / 64})">
    <path fill="url(#mark-fill)" fill-rule="evenodd" d="${glyph}"/>
  </g>`
}

function badge(x, y, label, kind) {
  const colors = {
    accent: ['#e6f0fc', '#0061cc'],
    muted: ['#e5e5ea', '#5f5f64'],
  }[kind]
  const width = Math.round(textWidth(label, 13, 500) + 20)
  return {
    width,
    svg: `<rect x="${x}" y="${y}" width="${width}" height="24" rx="12" fill="${colors[0]}"/>
    <text x="${x + width / 2}" y="${y + 16.5}" font-size="13" font-weight="500" fill="${colors[1]}" text-anchor="middle">${esc(label)}</text>`,
  }
}

function field(x, y, width, label, value, height = 40, lines = [value]) {
  const text = lines
    .map(
      (line, index) =>
        `<text x="${x + 14}" y="${y + 50 + index * 20}" font-size="15" fill="#1c1c1e">${esc(line)}</text>`,
    )
    .join('')
  return `<text x="${x}" y="${y + 12}" font-size="12" font-weight="700" fill="#6e6e73" letter-spacing="0.5">${esc(label.toUpperCase())}</text>
    <rect x="${x + 0.5}" y="${y + 22.5}" width="${width - 1}" height="${height}" rx="9" fill="#f6f6f6" stroke="#d1d1d6"/>
    ${text}`
}

function window(copy) {
  const x = 652
  const y = 64
  const w = 492
  const cardX = x + 16
  const cardY = y + 36 + 16
  const cardW = w - 32
  const inner = cardX + 20
  const innerW = cardW - 40
  let cy = cardY + 20

  const parts = []
  parts.push(
    `<text x="${inner}" y="${cy + 17}" font-size="18" font-weight="700" fill="#1c1c1e">${esc(copy.get('mock.title'))}</text>`,
  )
  cy += 34
  let bx = inner
  for (const [key, kind] of [
    ['mock.langActive', 'accent'],
    ['mock.langOther', 'muted'],
    ['mock.target', 'muted'],
  ]) {
    const b = badge(bx, cy, copy.get(key), kind)
    parts.push(b.svg)
    bx += b.width + 6
  }
  cy += 30
  bx = inner
  for (const key of ['mock.deck', 'mock.model']) {
    const b = badge(bx, cy, copy.get(key), 'muted')
    parts.push(b.svg)
    bx += b.width + 6
  }
  cy += 42

  const half = (innerW - 16) / 2
  parts.push(field(inner, cy, half - 46, copy.get('mock.front'), copy.get('mock.frontValue')))
  parts.push(
    `<rect x="${inner + half - 40}" y="${cy + 22}" width="40" height="40" rx="9" fill="#e5e5ea"/>
    <path d="M${inner + half - 24} ${cy + 36} l9 6 -9 6z" fill="#1c1c1e"/>`,
  )
  parts.push(field(inner + half + 16, cy, half, copy.get('mock.back'), copy.get('mock.backValue')))
  cy += 78

  const example = wrap(copy.get('mock.exampleValue'), 46)
  parts.push(
    field(inner, cy, innerW, copy.get('mock.example'), '', 62, example.length ? example : ['']),
  )
  cy += 100
  const translation = wrap(copy.get('mock.exampleTargetValue'), 46)
  parts.push(field(inner, cy, innerW, copy.get('mock.exampleTarget'), '', 62, translation))
  cy += 104

  const approve = copy.get('mock.approve')
  const discard = copy.get('mock.discard')
  const aw = Math.round(textWidth(approve, 15, 500) + 36)
  const dw = Math.round(textWidth(discard, 15, 500) + 36)
  const ax = inner + innerW - aw
  const dx = ax - 12 - dw
  parts.push(
    `<rect x="${dx}" y="${cy}" width="${dw}" height="38" rx="9" fill="#e5e5ea"/>
    <text x="${dx + dw / 2}" y="${cy + 24.5}" font-size="15" font-weight="500" fill="#1c1c1e" text-anchor="middle">${esc(discard)}</text>
    <rect x="${ax}" y="${cy}" width="${aw}" height="38" rx="9" fill="#0a6fe0"/>
    <text x="${ax + aw / 2}" y="${cy + 24.5}" font-size="15" font-weight="500" fill="#fff" text-anchor="middle">${esc(approve)}</text>`,
  )
  cy += 38 + 20

  const cardH = cy - cardY
  const h = cardH + 36 + 32
  return `<g>
    <rect x="${x}" y="${y + 18}" width="${w}" height="${h}" rx="16" fill="${INK}" opacity="0.16" filter="url(#blur)"/>
    <rect x="${x}" y="${y}" width="${w}" height="${h}" rx="14" fill="#f6f6f6" stroke="#000" stroke-opacity="0.14"/>
    <path d="M${x} ${y + 14} a14 14 0 0 1 14 -14 h${w - 28} a14 14 0 0 1 14 14 v22 h-${w} z" fill="#ebebeb"/>
    <line x1="${x}" y1="${y + 36.5}" x2="${x + w}" y2="${y + 36.5}" stroke="#000" stroke-opacity="0.08"/>
    <circle cx="${x + 20}" cy="${y + 18}" r="6.5" fill="#ff5f57"/>
    <circle cx="${x + 41}" cy="${y + 18}" r="6.5" fill="#febc2e"/>
    <circle cx="${x + 62}" cy="${y + 18}" r="6.5" fill="#28c840"/>
    <rect x="${cardX + 0.5}" y="${cardY + 0.5}" width="${cardW - 1}" height="${cardH}" rx="12" fill="#fff" stroke="#d1d1d6"/>
    ${parts.join('\n    ')}
  </g>`
}

/**
 * The headline with its selected word: `title_html` marks the word with a
 * <span>; the rest of the markup is dropped.
 */
function parseTitle(html) {
  const match = /^(.*?)<span class="hero-selection">(.*?)<\/span>(.*)$/.exec(html)
  if (!match) throw new Error('hero.title_html has no <span class="hero-selection">')
  const [, before, word, after] = match
  const plain = (text) => text.replace(/<[^>]+>/g, '')
  return { text: plain(before + word + after), word: plain(word) }
}

const TITLE = { x: 72, y: 268, size: 56, weight: 800, spacing: -1.6, leading: 64, wrap: 17 }
/** The caret after the selected word: its gap to the word's ink and its width. */
const CARET = { offset: 4, width: 4.5 }

/**
 * One headline line as <text>, with the selected word in its own <tspan>.
 * `restX` starts the text after the word (its leading space included) at the
 * caret's right edge, as on the page, where the caret is the word's border and
 * a full space follows it. Without it the caret would sit inside that space.
 */
function titleLine(line, index, word, { probe = false, restX } = {}) {
  const y = TITLE.y + index * TITLE.leading
  const attrs = `xml:space="preserve" x="${TITLE.x}" y="${y}" font-size="${TITLE.size}" font-weight="${TITLE.weight}" letter-spacing="${TITLE.spacing}"`
  const at = word ? line.indexOf(word) : -1
  if (at === -1) {
    return `<text ${attrs} fill="${probe ? 'none' : INK}">${esc(line)}</text>`
  }
  const rest = probe ? 'fill-opacity="0"' : ''
  const restAt = restX === undefined ? '' : ` x="${restX}"`
  return `<text ${attrs} fill="${INK}"><tspan ${rest}>${esc(line.slice(0, at))}</tspan><tspan>${esc(word)}</tspan><tspan${restAt} ${rest}>${esc(line.slice(at + word.length))}</tspan></text>`
}

/**
 * Reads an 8-bit RGBA PNG (what rsvg-convert writes) into its width, height
 * and pixel bytes.
 */
function decodePng(buffer) {
  let offset = 8
  let width = 0
  let height = 0
  const data = []
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset)
    const type = buffer.toString('ascii', offset + 4, offset + 8)
    const chunk = buffer.subarray(offset + 8, offset + 8 + length)
    if (type === 'IHDR') {
      width = chunk.readUInt32BE(0)
      height = chunk.readUInt32BE(4)
      if (chunk[8] !== 8 || chunk[9] !== 6 || chunk[12] !== 0) {
        throw new Error('expected a non-interlaced 8-bit RGBA PNG from rsvg-convert')
      }
    }
    if (type === 'IDAT') data.push(chunk)
    offset += 12 + length
  }
  const raw = inflateSync(Buffer.concat(data))
  const stride = width * 4
  const pixels = Buffer.alloc(stride * height)
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]
    for (let x = 0; x < stride; x++) {
      const value = raw[y * (stride + 1) + 1 + x]
      const left = x >= 4 ? pixels[y * stride + x - 4] : 0
      const up = y > 0 ? pixels[(y - 1) * stride + x] : 0
      const upLeft = x >= 4 && y > 0 ? pixels[(y - 1) * stride + x - 4] : 0
      let predictor = 0
      if (filter === 1) predictor = left
      else if (filter === 2) predictor = up
      else if (filter === 3) predictor = (left + up) >> 1
      else if (filter === 4) {
        const p = left + up - upLeft
        const [pa, pb, pc] = [Math.abs(p - left), Math.abs(p - up), Math.abs(p - upLeft)]
        predictor = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft
      }
      pixels[y * stride + x] = (value + predictor) & 0xff
    }
  }
  return { width, height, pixels }
}

/**
 * Where rsvg-convert draws the selected word: the line is rendered with only
 * that word visible and the inked columns are measured, so the selection and
 * the caret fit the word in whatever font librsvg picks.
 */
function measureWord(line, index, word, work) {
  const probe = path.join(work, 'probe.svg')
  const png = path.join(work, 'probe.png')
  writeFileSync(
    probe,
    `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630" font-family="${FONT}">
  ${titleLine(line, index, word, { probe: true })}
</svg>
`,
  )
  execFileSync('rsvg-convert', ['-w', '1200', '-h', '630', probe, '-o', png])
  const { width, height, pixels } = decodePng(readFileSync(png))
  let left = width
  let right = -1
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (pixels[(y * width + x) * 4 + 3] > 96) {
        left = Math.min(left, x)
        right = Math.max(right, x)
      }
    }
  }
  if (right < left) throw new Error(`could not measure the selected word "${word}"`)
  return { left, right: right + 1 }
}

function ogSvg(copy, glyph, work) {
  const { text, word } = parseTitle(copy.get('hero.title_html'))
  const title = wrap(text, TITLE.wrap)
  const wordLine = title.findIndex((line) => line.includes(word))
  const eyebrow = copy.get('hero.eyebrow')
  const eyebrowWidth = Math.round(textWidth(eyebrow, 17, 600) + 32)
  let selection = ''
  let restX
  if (wordLine !== -1) {
    const { left, right } = measureWord(title[wordLine], wordLine, word, work)
    const base = TITLE.y + wordLine * TITLE.leading
    const top = base - Math.round(TITLE.size * 0.84)
    const height = Math.round(TITLE.size * 1.08)
    const x = left - 5
    const caret = right + CARET.offset
    restX = caret + CARET.width
    selection = `<path d="M${x + 6} ${top}H${caret}V${top + height}H${x + 6}A6 6 0 0 1 ${x} ${top + height - 6}V${top + 6}A6 6 0 0 1 ${x + 6} ${top}Z" fill="url(#selection)"/>
  <rect x="${caret}" y="${top}" width="${CARET.width}" height="${height}" fill="${INK}"/>`
  }
  const titleLines = title
    .map((line, index) => titleLine(line, index, index === wordLine ? word : null, { restX }))
    .join('\n  ')
  const keysY = TITLE.y + title.length * TITLE.leading + 8
  const keys = ['⇧', '⌘', 'C']
    .map(
      (key, index) => `<g transform="translate(${72 + index * 60} ${keysY})">
      <rect y="4" width="50" height="50" rx="12" fill="${BORDER_STRONG}"/>
      <rect x="0.5" y="0.5" width="49" height="49" rx="11.5" fill="#ffffff" stroke="${BORDER_STRONG}"/>
      <text x="25" y="33" font-size="22" font-weight="600" fill="${INK}" text-anchor="middle">${key}</text>
    </g>`,
    )
    .join('\n  ')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630" font-family="${FONT}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="1" stop-color="#f8f2fb"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="${CORAL}" stop-opacity="0.34"/>
      <stop offset="1" stop-color="${CORAL}" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="glow-2" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="${SELECTION[1]}" stop-opacity="0.22"/>
      <stop offset="1" stop-color="${SELECTION[1]}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="mark-fill" x1="28" y1="8" x2="36" y2="52" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${CORAL}"/>
      <stop offset="1" stop-color="${RASPBERRY}"/>
    </linearGradient>
    <linearGradient id="selection" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="${SELECTION[0]}"/>
      <stop offset="1" stop-color="${SELECTION[1]}"/>
    </linearGradient>
    <filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="22"/></filter>
  </defs>
  <rect width="1200" height="630" fill="url(#bg)"/>
  <circle cx="960" cy="250" r="430" fill="url(#glow)"/>
  <circle cx="760" cy="560" r="320" fill="url(#glow-2)"/>
  ${mark(glyph, 68, 58, 64)}
  <text x="140" y="101" font-size="30" font-weight="800" fill="${INK}" letter-spacing="-0.6">Polywort</text>
  <rect x="72" y="158" width="${eyebrowWidth}" height="38" rx="19" fill="${ACCENT_SOFT}" stroke="${ACCENT_BORDER}"/>
  <text x="${72 + eyebrowWidth / 2}" y="183" font-size="17" font-weight="600" fill="${ACCENT_TEXT}" text-anchor="middle">${esc(eyebrow)}</text>
  ${selection}
  ${titleLines}
  ${keys}
  ${window(copy)}
</svg>
`
}

/**
 * The touch icon: branding/icon.svg cropped to the icon's 824-unit body, with
 * the body square and without its drop shadow, so the colour reaches every
 * corner. Fails if the icon's layout changed and the edit no longer applies.
 */
function touchIconSvg(icon) {
  const edits = [
    [
      'width="1024" height="1024" viewBox="0 0 1024 1024"',
      'width="180" height="180" viewBox="100 100 824 824"',
    ],
    [' filter="url(#icon-bodyShadow)"', ''],
    [/(<rect x="100" y="100" width="824" height="824") rx="185"/g, '$1'],
  ]
  let svg = icon
  for (const [from, to] of edits) {
    const next = svg.replace(from, to)
    if (next === svg) throw new Error(`branding/icon.svg changed: cannot apply ${from}`)
    svg = next
  }
  return svg
}

function flatten(tree, prefix = '', out = new Map()) {
  for (const [key, value] of Object.entries(tree)) {
    const name = prefix ? `${prefix}.${key}` : key
    if (typeof value === 'object') flatten(value, name, out)
    else out.set(name, value)
  }
  return out
}

async function main() {
  const { values } = parseArgs({ options: { 'keep-svg': { type: 'string' } } })
  const work = values['keep-svg']
    ? path.resolve(values['keep-svg'])
    : await mkdtemp(path.join(os.tmpdir(), 'og-'))
  const icon = await readFile(path.join(REPO_ROOT, 'branding', 'icon.svg'), 'utf8')
  const partial = await readFile(path.join(SITE_DIR, 'src', 'partials', 'mark.svg'), 'utf8')
  const glyph = /\sd="([^"]+)"/.exec(partial)?.[1].replace(/\s+/g, ' ')
  if (!glyph) throw new Error('src/partials/mark.svg has no path')
  // The body colour as background as well, so the touch icon is opaque even
  // where an antialiased edge would leave a pixel partly transparent.
  const jobs = [['apple-touch-icon', touchIconSvg(icon), 180, 180, ['-b', RASPBERRY]]]
  for (const lang of ['en', 'de']) {
    const copy = flatten(
      JSON.parse(await readFile(path.join(SITE_DIR, 'src', 'i18n', `${lang}.json`), 'utf8')),
    )
    jobs.push([`og-${lang}`, ogSvg(copy, glyph, work), 1200, 630, []])
  }
  for (const [name, svg, width, height, extra] of jobs) {
    const svgFile = path.join(work, `${name}.svg`)
    await writeFile(svgFile, svg)
    const png = path.join(ASSETS, `${name}.png`)
    execFileSync('rsvg-convert', [
      '-w',
      String(width),
      '-h',
      String(height),
      ...extra,
      svgFile,
      '-o',
      png,
    ])
    console.log(`wrote ${path.relative(process.cwd(), png)}`)
  }
  if (!values['keep-svg']) await rm(work, { recursive: true, force: true })
}

main().catch((error) => {
  console.error(error.message)
  process.exit(1)
})
