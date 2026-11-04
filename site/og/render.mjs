#!/usr/bin/env node
// Draws the social preview images (site/src/assets/og-en.png, og-de.png) and
// the touch icon (apple-touch-icon.png) from the site's own artwork: the mark
// and a drawing of the approval window with the app's real labels, taken from
// the page copy in site/src/i18n. Never from the Tauri template icon.
//
// The build does not run this: the PNGs are committed. Run it again after the
// copy or the mark changes. It needs `rsvg-convert` (librsvg, e.g.
// `brew install librsvg`) and works offline.
//
//   node site/og/render.mjs [--keep-svg <dir>]

import { execFileSync } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const SITE_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const ASSETS = path.join(SITE_DIR, 'src', 'assets')

const FONT = "'Helvetica Neue', Helvetica, Arial, sans-serif"
const ACCENT = '#4f46e5'

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

function mark(x, y, size) {
  const s = size / 32
  return `<g transform="translate(${x} ${y}) scale(${s})">
    <rect width="32" height="32" rx="8" fill="${ACCENT}"/>
    <rect x="9.5" y="5" width="17" height="12.5" rx="2.5" fill="#fff" fill-opacity="0.42" transform="rotate(-9 18 11.25)"/>
    <rect x="5.5" y="10" width="21" height="15.5" rx="3" fill="#fff"/>
    <rect x="8.75" y="13.75" width="9.5" height="4" rx="1.1" fill="${ACCENT}" fill-opacity="0.3"/>
    <rect x="18.75" y="12.75" width="1.4" height="6" rx="0.7" fill="${ACCENT}"/>
    <rect x="8.75" y="20.75" width="14" height="1.9" rx="0.95" fill="#d3d6e4"/>
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
    <rect x="${x}" y="${y + 18}" width="${w}" height="${h}" rx="16" fill="#1e2246" opacity="0.16" filter="url(#blur)"/>
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

function ogSvg(copy) {
  const title = wrap(copy.get('hero.title'), 17)
  const eyebrow = copy.get('hero.eyebrow')
  const eyebrowWidth = Math.round(textWidth(eyebrow, 17, 600) + 32)
  const titleY = 268
  const titleLines = title
    .map(
      (line, index) =>
        `<text x="72" y="${titleY + index * 64}" font-size="56" font-weight="800" fill="#13151b" letter-spacing="-1.6">${esc(line)}</text>`,
    )
    .join('\n  ')
  const keysY = titleY + title.length * 64 + 8
  const keys = ['⇧', '⌘', 'C']
    .map(
      (key, index) => `<g transform="translate(${72 + index * 60} ${keysY})">
      <rect y="4" width="50" height="50" rx="12" fill="#cfd4df"/>
      <rect x="0.5" y="0.5" width="49" height="49" rx="11.5" fill="#ffffff" stroke="#cfd4df"/>
      <text x="25" y="33" font-size="22" font-weight="600" fill="#13151b" text-anchor="middle">${key}</text>
    </g>`,
    )
    .join('\n  ')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630" font-family="${FONT}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="1" stop-color="#eef0fe"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#6366f1" stop-opacity="0.28"/>
      <stop offset="1" stop-color="#6366f1" stop-opacity="0"/>
    </radialGradient>
    <filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="22"/></filter>
  </defs>
  <rect width="1200" height="630" fill="url(#bg)"/>
  <circle cx="930" cy="300" r="420" fill="url(#glow)"/>
  ${mark(72, 64, 52)}
  <text x="140" y="99" font-size="27" font-weight="700" fill="#13151b" letter-spacing="-0.3">Vocabulary Automator</text>
  <rect x="72" y="158" width="${eyebrowWidth}" height="38" rx="19" fill="#eef0fe" stroke="#c9ccf8"/>
  <text x="${72 + eyebrowWidth / 2}" y="183" font-size="17" font-weight="600" fill="#4338ca" text-anchor="middle">${esc(eyebrow)}</text>
  ${titleLines}
  ${keys}
  ${window(copy)}
</svg>
`
}

function touchIconSvg() {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="180" height="180" viewBox="0 0 32 32">
  <rect width="32" height="32" fill="${ACCENT}"/>
  <g transform="translate(3.2 3.2) scale(0.8)">
    <rect x="9.5" y="5" width="17" height="12.5" rx="2.5" fill="#fff" fill-opacity="0.42" transform="rotate(-9 18 11.25)"/>
    <rect x="5.5" y="10" width="21" height="15.5" rx="3" fill="#fff"/>
    <rect x="8.75" y="13.75" width="9.5" height="4" rx="1.1" fill="${ACCENT}" fill-opacity="0.3"/>
    <rect x="18.75" y="12.75" width="1.4" height="6" rx="0.7" fill="${ACCENT}"/>
    <rect x="8.75" y="20.75" width="14" height="1.9" rx="0.95" fill="#d3d6e4"/>
  </g>
</svg>
`
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
  const jobs = [['apple-touch-icon', touchIconSvg(), 180, 180]]
  for (const lang of ['en', 'de']) {
    const copy = flatten(
      JSON.parse(await readFile(path.join(SITE_DIR, 'src', 'i18n', `${lang}.json`), 'utf8')),
    )
    jobs.push([`og-${lang}`, ogSvg(copy), 1200, 630])
  }
  for (const [name, svg, width, height] of jobs) {
    const svgFile = path.join(work, `${name}.svg`)
    await writeFile(svgFile, svg)
    const png = path.join(ASSETS, `${name}.png`)
    execFileSync('rsvg-convert', ['-w', String(width), '-h', String(height), svgFile, '-o', png])
    console.log(`wrote ${path.relative(process.cwd(), png)}`)
  }
  if (!values['keep-svg']) await rm(work, { recursive: true, force: true })
}

main().catch((error) => {
  console.error(error.message)
  process.exit(1)
})
