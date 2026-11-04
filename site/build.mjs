#!/usr/bin/env node
// Builds the static product site: renders src/page.html once per language
// (src/i18n/<lang>.json) plus 404.html, sitemap.xml and robots.txt, and copies
// the stylesheet, the script, the assets, vercel.json and the newest macOS disk
// image from site/downloads/. Node built-ins only, so it needs no npm install.
//
//   node site/build.mjs [--out <dir>] [--downloads <dir>] [--allow-version-mismatch]
//   SITE_URL=https://example.org node site/build.mjs
//
// The disk image must be the universal build of the app version in
// src-tauri/tauri.conf.json ("…_<version>_universal.dmg", Tauri's own name),
// because the page labels it with that version and "Apple Silicon and Intel".
// --allow-version-mismatch lets another version through, with a warning.
//
// Everything is rendered in memory first, so a failing build leaves the
// output folder as it was.
//
// Template syntax (src/page.html, src/404.html):
//   {{group.key}}     a copy value; HTML-escaped unless the key ends in _html
//   {{build.name}}    a value the build computes (URLs, version, the .dmg facts)
//   {{lang:key}}      a copy value of one language (only the 404 page uses it)
//   {{#flag}}…{{/flag}} / {{^flag}}…{{/flag}}  kept when the flag is set / unset
//   {{>name}}         the file src/partials/<name>.svg
//   {{icon:name}}     an inline icon from src/icons.json
// Copy values may contain {token}s such as {version}; build() lists them.

import { createHash } from 'node:crypto'
import { createReadStream, realpathSync } from 'node:fs'
import { copyFile, cp, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

export const SITE_DIR = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(SITE_DIR, '..')
const SRC_DIR = path.join(SITE_DIR, 'src')

export const DEFAULT_SITE_URL = 'https://vocabulary-automator.vercel.app'
export const REPO_URL = 'https://github.com/fabsGitHub/vocabulary-anki-automator'
export const RELEASES_URL = `${REPO_URL}/releases`
export const ANKI_URL = 'https://apps.ankiweb.net/'
export const MYMEMORY_URL = 'https://mymemory.translated.net/'

/** Every language the site is published in; the first one is the default at `/`. */
export const LOCALES = {
  en: { path: '/', file: 'index.html', ogLocale: 'en_US', numberLocale: 'en-US' },
  de: { path: '/de/', file: 'de/index.html', ogLocale: 'de_DE', numberLocale: 'de-DE' },
}

const ESCAPES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }

export function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (char) => ESCAPES[char])
}

/** Nested copy objects as one flat map: `{ hero: { title } }` → `hero.title`. */
export function flatten(tree, prefix = '') {
  const flat = new Map()
  for (const [key, value] of Object.entries(tree)) {
    const name = prefix ? `${prefix}.${key}` : key
    if (value !== null && typeof value === 'object') {
      for (const [inner, text] of flatten(value, name)) flat.set(inner, text)
    } else if (typeof value === 'string') {
      flat.set(name, value)
    } else {
      throw new Error(`Copy value ${name} is not a string`)
    }
  }
  return flat
}

/** Loads one language's copy from src/i18n/<lang>.json. */
export async function loadCopy(lang, srcDir = SRC_DIR) {
  const file = path.join(srcDir, 'i18n', `${lang}.json`)
  return flatten(JSON.parse(await readFile(file, 'utf8')))
}

/** The keys that only one of two copy maps has, per side. */
export function keyDifference(a, b) {
  return {
    onlyInFirst: [...a.keys()].filter((key) => !b.has(key)),
    onlyInSecond: [...b.keys()].filter((key) => !a.has(key)),
  }
}

/**
 * Formats a copy value: fills in its {tokens} and escapes it, unless the key
 * ends in `_html`, in which case the value is markup and only the tokens are
 * escaped. An unknown token is a typo in the copy, so it fails the build.
 */
export function formatCopy(key, value, tokens) {
  const isHtml = key.endsWith('_html')
  const filled = value.replace(/\{([A-Za-z][A-Za-z0-9]*)\}/g, (_, name) => {
    if (!(name in tokens)) throw new Error(`Copy ${key} uses the unknown token {${name}}`)
    return isHtml ? escapeHtml(tokens[name]) : tokens[name]
  })
  return isHtml ? filled : escapeHtml(filled)
}

/**
 * Renders a template. `lookup(key)` returns the HTML for `{{key}}` or
 * `{{lang:key}}`, `flags` decides the sections, `partials` and `icons` supply
 * the includes. Anything left unresolved throws, so no `{{` reaches the output.
 */
export function renderTemplate(template, { lookup, flags, partials, icons }) {
  // A line of nothing but section tags (Prettier joins `{{/a}} {{^a}}`) takes
  // its line break and spaces with it, as standalone tags do in Mustache.
  let html = template.replace(/^[ \t]*((?:\{\{[#^/][\w.-]+\}\}[ \t]*)+)\r?\n/gm, (_, tags) =>
    tags.replace(/[ \t]+/g, ''),
  )
  html = html.replace(/^([ \t]*)\{\{>\s*([\w-]+)\s*\}\}/gm, (_, indent, name) => {
    if (!(name in partials)) throw new Error(`Unknown partial {{>${name}}}`)
    return partials[name]
      .trim()
      .split('\n')
      .map((line) => indent + line)
      .join('\n')
  })
  html = html.replace(/\{\{>\s*([\w-]+)\s*\}\}/g, (_, name) => {
    if (!(name in partials)) throw new Error(`Unknown partial {{>${name}}}`)
    return partials[name].trim()
  })
  const section = /\{\{([#^])([\w.-]+)\}\}([\s\S]*?)\{\{\/\2\}\}/g
  for (let previous = null; previous !== html;) {
    previous = html
    html = html.replace(section, (_, kind, name, body) => {
      if (!(name in flags)) throw new Error(`Unknown section flag {{${kind}${name}}}`)
      return Boolean(flags[name]) === (kind === '#') ? body : ''
    })
  }
  html = html.replace(/\{\{icon:([\w-]+)\}\}/g, (_, name) => {
    if (!(name in icons)) throw new Error(`Unknown icon {{icon:${name}}}`)
    return (
      '<svg class="icon" viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" ' +
      'focusable="false" fill="none" stroke="currentColor" stroke-width="1.75" ' +
      `stroke-linecap="round" stroke-linejoin="round">${icons[name]}</svg>`
    )
  })
  html = html.replace(/\{\{\s*([\w.:-]+)\s*\}\}/g, (_, key) => lookup(key))
  const left = html.indexOf('{{')
  if (left !== -1) {
    throw new Error(`Unresolved template syntax near: ${html.slice(left, left + 60)}`)
  }
  return html
}

/**
 * A vercel.json `source` such as `/downloads/(.*\.dmg)` as a RegExp: the
 * groups are regular expressions, the rest is literal. The preview server
 * applies the headers with it and the tests check the rules against the URLs
 * the build publishes, so both read the patterns the same way.
 */
export function sourcePattern(source) {
  let pattern = ''
  let depth = 0
  for (const char of source) {
    if (char === '(') depth += 1
    if (depth > 0) pattern += char
    else pattern += char.replace(/[.*+?^${}|[\]\\]/g, '\\$&')
    if (char === ')') depth -= 1
  }
  return new RegExp(`^${pattern}$`)
}

/** The newest `.dmg` in `dir` (by modification time), or null when there is none. */
export async function findNewestDmg(dir) {
  let names
  try {
    names = await readdir(dir)
  } catch (error) {
    if (error.code === 'ENOENT') return null
    throw error
  }
  const candidates = []
  for (const name of names) {
    if (!name.toLowerCase().endsWith('.dmg')) continue
    const file = path.join(dir, name)
    const info = await stat(file)
    if (info.isFile()) candidates.push({ name, file, size: info.size, mtime: info.mtimeMs })
  }
  candidates.sort((a, b) => b.mtime - a.mtime || a.name.localeCompare(b.name))
  return candidates[0] ?? null
}

/**
 * The file name the disk image is published under: Tauri names it
 * "Vocabulary Automator_0.1.0_universal.dmg", and a space would need quoting in
 * the `shasum` command the page shows and escaping in the URL.
 */
export function publishedDmgName(name) {
  return name.replace(/[^A-Za-z0-9._-]+/g, '-')
}

/**
 * Checks that a disk image's file name says what the page says about it:
 * Tauri names a universal build "<product>_<version>_universal.dmg". A
 * per-architecture image, or one of another version, would be published with
 * a false label, so it fails the build. With `allowVersionMismatch`, another
 * version passes and the warning is returned instead; the page then still
 * shows `version`.
 */
export function checkDmgName(name, version, { allowVersionMismatch = false } = {}) {
  const expected = `_${version}_universal.dmg`
  if (!/_universal\.dmg$/i.test(name)) {
    throw new Error(
      `${name} is not the universal disk image (Apple Silicon and Intel) the page describes: ` +
        `expected a name ending in ${expected} (see site/README.md)`,
    )
  }
  if (name.toLowerCase().endsWith(expected.toLowerCase())) return null
  const message = `${name} does not carry the app version ${version} from src-tauri/tauri.conf.json (expected a name ending in ${expected})`
  if (!allowVersionMismatch) {
    throw new Error(`${message}; pass --allow-version-mismatch to publish it anyway`)
  }
  return `warning: ${message}; the page still says version ${version}`
}

export async function sha256File(file) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest('hex')
}

/**
 * A size in megabytes (10^6 bytes, as Finder counts them) with one decimal,
 * joined to its unit by a no-break space so the two never land on different
 * lines.
 */
export function formatMegabytes(bytes, numberLocale) {
  const number = new Intl.NumberFormat(numberLocale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(bytes / 1_000_000)
  return `${number}\u00a0MB`
}

/**
 * The site's origin from SITE_URL, without a trailing slash. Every link in the
 * page is root-relative (/styles.css, /de/, /downloads/…), so the site cannot
 * live under a path, and a SITE_URL with one is rejected.
 */
export function normalizeSiteUrl(raw) {
  const value = (raw ?? '').trim() || DEFAULT_SITE_URL
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error(`SITE_URL is not a URL: ${value}`)
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') {
    throw new Error(`SITE_URL must be an http(s) URL: ${value}`)
  }
  if (url.search || url.hash) throw new Error(`SITE_URL must not have a query or hash: ${value}`)
  if (url.pathname !== '/' || url.username || url.password) {
    throw new Error(`SITE_URL must be an origin, such as https://example.org: ${value}`)
  }
  return url.origin
}

/** The app version, from src-tauri/tauri.conf.json. */
export async function readVersion() {
  const conf = JSON.parse(
    await readFile(path.join(REPO_ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8'),
  )
  if (typeof conf.version !== 'string') throw new Error('src-tauri/tauri.conf.json has no version')
  return conf.version
}

async function readPartials(srcDir) {
  const dir = path.join(srcDir, 'partials')
  const partials = {}
  for (const name of await readdir(dir)) {
    if (name.endsWith('.svg'))
      partials[name.slice(0, -4)] = await readFile(path.join(dir, name), 'utf8')
  }
  return partials
}

async function exists(file) {
  try {
    await stat(file)
    return true
  } catch (error) {
    if (error.code === 'ENOENT') return false
    throw error
  }
}

/** Kept when the output folder is emptied: the Vercel CLI's link to its project. */
const KEEP_IN_OUT_DIR = new Set(['.vercel'])

/**
 * Empties `outDir` before a build, except for a `.vercel` link, but refuses to
 * wipe a directory that holds something other than an earlier build (a
 * mistyped `--out`).
 */
async function prepareOutDir(outDir) {
  if (await exists(outDir)) {
    const entries = (await readdir(outDir)).filter((name) => !KEEP_IN_OUT_DIR.has(name))
    const earlierBuild = entries.includes('index.html') && entries.includes('vercel.json')
    if (entries.length > 0 && !earlierBuild) {
      throw new Error(`Refusing to empty ${outDir}: it is not empty and holds no earlier build`)
    }
    for (const name of entries) await rm(path.join(outDir, name), { recursive: true, force: true })
  }
  await mkdir(outDir, { recursive: true })
}

function sitemap(siteUrl) {
  const urls = Object.values(LOCALES).map(({ path: page }) => `${siteUrl}${page}`)
  const alternates = Object.entries(LOCALES)
    .map(
      ([lang, { path: page }]) =>
        `    <xhtml:link rel="alternate" hreflang="${lang}" href="${escapeHtml(siteUrl + page)}"/>`,
    )
    .concat(
      `    <xhtml:link rel="alternate" hreflang="x-default" href="${escapeHtml(siteUrl + '/')}"/>`,
    )
    .join('\n')
  const entries = urls
    .map((url) => `  <url>\n    <loc>${escapeHtml(url)}</loc>\n${alternates}\n  </url>`)
    .join('\n')
  return (
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" ' +
    'xmlns:xhtml="http://www.w3.org/1999/xhtml">\n' +
    `${entries}\n</urlset>\n`
  )
}

/**
 * Builds the site into `outDir` and returns what it did. Every option has the
 * default the CLI uses; tests pass a temporary `outDir` and `downloadsDir`,
 * and `srcDir` to build from a modified copy of src/.
 *
 * Everything that can fail (the copy, the templates, the disk image check and
 * its checksum) runs before `outDir` is touched, so a failed build leaves an
 * earlier one in place and the next build can replace it.
 */
export async function build({
  outDir = path.join(SITE_DIR, 'dist'),
  downloadsDir = path.join(SITE_DIR, 'downloads'),
  siteUrl = process.env.SITE_URL,
  srcDir = SRC_DIR,
  allowVersionMismatch = false,
} = {}) {
  const site = normalizeSiteUrl(siteUrl)
  const version = await readVersion()
  const warnings = []
  const copies = {}
  for (const lang of Object.keys(LOCALES)) copies[lang] = await loadCopy(lang, srcDir)

  const [first, ...others] = Object.keys(LOCALES)
  for (const lang of others) {
    const { onlyInFirst, onlyInSecond } = keyDifference(copies[first], copies[lang])
    if (onlyInFirst.length || onlyInSecond.length) {
      throw new Error(
        `${first}.json and ${lang}.json differ: only in ${first}: [${onlyInFirst}], ` +
          `only in ${lang}: [${onlyInSecond}]`,
      )
    }
  }

  const source = await findNewestDmg(downloadsDir)
  let dmg = null
  if (source) {
    const warning = checkDmgName(source.name, version, { allowVersionMismatch })
    if (warning) warnings.push(warning)
    const file = publishedDmgName(source.name)
    dmg = {
      file,
      source: source.name,
      bytes: source.size,
      sha256: await sha256File(source.file),
      href: `/downloads/${file}`,
    }
  }

  /** Every file the build writes, by its path in `outDir`, rendered in memory. */
  const outputs = new Map()
  outputs.set('vercel.json', await readFile(path.join(SITE_DIR, 'vercel.json'), 'utf8'))

  const partials = await readPartials(srcDir)
  const icons = JSON.parse(await readFile(path.join(srcDir, 'icons.json'), 'utf8'))
  const pageTemplate = await readFile(path.join(srcDir, 'page.html'), 'utf8')
  const langs = Object.keys(LOCALES)
  const pages = []

  for (const lang of langs) {
    const locale = LOCALES[lang]
    const altLang = langs.find((other) => other !== lang)
    const ogFile = `assets/og-${lang}.png`
    const og = await exists(path.join(srcDir, ogFile))
    const dmgSize = dmg ? formatMegabytes(dmg.bytes, locale.numberLocale) : ''
    const tokens = {
      version,
      dmgFile: dmg?.file ?? '',
      dmgSize,
      ankiUrl: ANKI_URL,
      mymemoryUrl: MYMEMORY_URL,
    }
    const values = {
      lang,
      canonical: `${site}${locale.path}`,
      enUrl: `${site}${LOCALES.en.path}`,
      deUrl: `${site}${LOCALES.de.path}`,
      homePath: locale.path,
      altLang,
      altPath: LOCALES[altLang].path,
      ogLocale: locale.ogLocale,
      ogLocaleAlt: LOCALES[altLang].ogLocale,
      ogImage: `${site}/${ogFile}`,
      repoUrl: REPO_URL,
      releasesUrl: RELEASES_URL,
      version,
      dmgHref: dmg?.href ?? '',
      dmgFile: dmg?.file ?? '',
      dmgSize,
      dmgSha256: dmg?.sha256 ?? '',
    }
    const lookup = (key) => {
      if (key.startsWith('build.')) {
        const name = key.slice('build.'.length)
        if (!(name in values)) throw new Error(`Unknown build value {{${key}}}`)
        return escapeHtml(values[name])
      }
      if (!copies[lang].has(key)) throw new Error(`Unknown copy key {{${key}}} (${lang})`)
      return formatCopy(key, copies[lang].get(key), tokens)
    }
    outputs.set(
      locale.file,
      renderTemplate(pageTemplate, {
        lookup,
        flags: { dmg: Boolean(dmg), og },
        partials,
        icons,
      }),
    )
    pages.push(locale.file)
  }

  const notFound = renderTemplate(await readFile(path.join(srcDir, '404.html'), 'utf8'), {
    lookup: (key) => {
      const match = /^(\w+):(.+)$/.exec(key)
      if (!match || !copies[match[1]]?.has(match[2])) {
        throw new Error(`Unknown 404 copy key {{${key}}}`)
      }
      return formatCopy(match[2], copies[match[1]].get(match[2]), { version })
    },
    flags: {},
    partials,
    icons,
  })
  outputs.set('404.html', notFound)
  pages.push('404.html')

  outputs.set('sitemap.xml', sitemap(site))
  outputs.set('robots.txt', `User-agent: *\nAllow: /\n\nSitemap: ${site}/sitemap.xml\n`)

  // Only now is the output folder emptied. vercel.json and the pages go first,
  // so even an interrupted copy below leaves a folder the next build recognizes.
  await prepareOutDir(outDir)
  for (const [file, content] of outputs) {
    const target = path.join(outDir, file)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, content)
  }
  await cp(path.join(srcDir, 'assets'), path.join(outDir, 'assets'), { recursive: true })
  await copyFile(path.join(srcDir, 'styles.css'), path.join(outDir, 'styles.css'))
  await copyFile(path.join(srcDir, 'main.js'), path.join(outDir, 'main.js'))
  if (source) {
    await mkdir(path.join(outDir, 'downloads'), { recursive: true })
    await copyFile(source.file, path.join(outDir, 'downloads', dmg.file))
  }

  return { outDir, siteUrl: site, version, dmg, pages, warnings }
}

async function main() {
  const { values } = parseArgs({
    options: {
      out: { type: 'string' },
      downloads: { type: 'string' },
      'allow-version-mismatch': { type: 'boolean' },
    },
  })
  const result = await build({
    outDir: values.out ? path.resolve(values.out) : undefined,
    downloadsDir: values.downloads ? path.resolve(values.downloads) : undefined,
    allowVersionMismatch: Boolean(values['allow-version-mismatch']),
  })
  for (const warning of result.warnings) console.warn(warning)
  const relative = path.relative(process.cwd(), result.outDir) || '.'
  console.log(`Built ${result.pages.join(', ')} for ${result.siteUrl} into ${relative}`)
  if (result.dmg) {
    console.log(`  ${result.dmg.file}: ${result.dmg.bytes} bytes, SHA-256 ${result.dmg.sha256}`)
  } else {
    console.log('  no .dmg in the downloads folder: the macOS button links to GitHub Releases')
  }
}

/**
 * True when Node runs this file as the script. Both sides are real paths:
 * import.meta.url is resolved through symlinks and argv[1] is not, so a
 * plain comparison would skip the build when it is called through a link.
 */
function isMainModule() {
  if (!process.argv[1]) return false
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
  } catch {
    return false
  }
}

if (isMainModule()) {
  main().catch((error) => {
    console.error(error.message)
    process.exit(1)
  })
}
