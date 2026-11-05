// Tests for the product site build (site/build.mjs). Each build goes into its
// own temporary directory with a fake .dmg (or none), so nothing here touches
// site/dist, site/downloads or the network.
//
//   npm run site:test

import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  symlink,
  utimes,
  writeFile,
} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { after, before, describe, test } from 'node:test'
import { promisify } from 'node:util'

import {
  ANKI_URL,
  MYMEMORY_URL,
  PRODUCT_NAME,
  RELEASES_URL,
  REPO_URL,
  SITE_DIR,
  build,
  checkDmgName,
  escapeHtml,
  formatCopy,
  formatMegabytes,
  keyDifference,
  loadCopy,
  normalizeSiteUrl,
  publishedDmgName,
  readVersion,
  renderTemplate,
  sourcePattern,
} from './build.mjs'

const SITE_URL = 'https://polywort.vercel.app'
/** The version the build reads, so a release PR's version bump keeps these tests green. */
const VERSION = await readVersion()
/** Tauri's name for the universal disk image; it has no space to replace. */
const DMG_NAME = `Polywort_${VERSION}_universal.dmg`
const DMG_PUBLISHED = DMG_NAME
/** What Tauri named the image before the app was called Polywort. */
const OLD_DMG_NAME = `Vocabulary Automator_${VERSION}_universal.dmg`
const DMG_BYTES = 2_456_789

/** The only external URLs a built page may contain, besides its own site URL. */
const ALLOWED_EXTERNAL = new Set([
  REPO_URL,
  RELEASES_URL,
  ANKI_URL,
  'https://ankiweb.net/shared/info/2055492159',
  MYMEMORY_URL,
])

const HTML_PAGES = ['index.html', 'de/index.html', '404.html']

const REPO_ROOT = path.resolve(SITE_DIR, '..')

/**
 * The message `group.key` of an app's i18n source (src/i18n/<lang>.ts), read
 * as text: a top-level group at two spaces, its keys at four, one quoted line.
 */
function appMessage(source, group, key) {
  const start = source.indexOf(`\n  ${group}: {\n`)
  assert.notEqual(start, -1, `no group ${group}`)
  const end = source.indexOf('\n  },', start)
  const value = new RegExp(`\\n    ${key}:\\s*'([^'\\n]*)',`).exec(source.slice(start, end))?.[1]
  assert.ok(value, `no message ${group}.${key}`)
  return value
}

const temporary = []

async function tempDir(prefix) {
  const dir = await mkdtemp(path.join(os.tmpdir(), `site-${prefix}-`))
  temporary.push(dir)
  return dir
}

/** A deterministic fake disk image of `bytes` bytes; returns its SHA-256. */
async function fakeDmg(dir, name, bytes, fill = 7) {
  await mkdir(dir, { recursive: true })
  const content = Buffer.alloc(bytes, fill)
  await writeFile(path.join(dir, name), content)
  return createHash('sha256').update(content).digest('hex')
}

async function readPages(outDir) {
  const pages = {}
  for (const page of HTML_PAGES) pages[page] = await readFile(path.join(outDir, page), 'utf8')
  return pages
}

/** Text of every `<tag …>` start tag in `html`. */
function startTags(html, tag) {
  return html.match(new RegExp(`<${tag}\\b[^>]*>`, 'gi')) ?? []
}

after(async () => {
  for (const dir of temporary) await rm(dir, { recursive: true, force: true })
})

describe('copy', () => {
  test('en.json and de.json have exactly the same keys', async () => {
    const [en, de] = await Promise.all([loadCopy('en'), loadCopy('de')])
    assert.deepEqual(keyDifference(en, de), { onlyInFirst: [], onlyInSecond: [] })
    assert.ok(en.size > 100, 'the copy is loaded')
  })

  test('no copy value is empty', async () => {
    for (const lang of ['en', 'de']) {
      for (const [key, value] of await loadCopy(lang)) {
        assert.notEqual(value.trim(), '', `${lang}: ${key} is empty`)
      }
    }
  })

  test('calls the app Polywort and names the old name only to explain it', async () => {
    assert.equal(PRODUCT_NAME, 'Polywort')
    for (const lang of ['en', 'de']) {
      const copy = await loadCopy(lang)
      for (const [key, value] of copy) {
        if (key === 'faq.q13.a_html') continue
        assert.ok(!value.includes('Vocabulary Automator'), `${lang}: ${key} uses the old name`)
      }
      assert.match(copy.get('faq.q13.a_html'), /Polywort.*Vocabulary Automator/s)
      assert.match(copy.get('meta.title'), /^Polywort – /)
    }
  })

  test('builds the disk image the app config names', async () => {
    // Tauri names the bundle and the disk image after productName, and
    // checkDmgName only lets Polywort_<version>_universal.dmg through.
    const conf = JSON.parse(
      await readFile(path.join(REPO_ROOT, 'src-tauri/tauri.conf.json'), 'utf8'),
    )
    assert.equal(conf.productName, PRODUCT_NAME)
  })

  test('names the recommended note type and its button as the app does', async () => {
    const noteType = /export const RECOMMENDED_NOTE_TYPE = '([^']+)'/.exec(
      await readFile(path.join(REPO_ROOT, 'src/domain/fieldMapping.ts'), 'utf8'),
    )?.[1]
    assert.equal(noteType, PRODUCT_NAME)
    for (const lang of ['en', 'de']) {
      const copy = await loadCopy(lang)
      const app = await readFile(path.join(REPO_ROOT, `src/i18n/${lang}.ts`), 'utf8')
      // The drawing shows the approval window's own note type line.
      const noteTypeLine = appMessage(app, 'approval', 'model').replace('{model}', noteType)
      assert.equal(copy.get('mock.model'), noteTypeLine, `${lang}: mock.model`)
      const button = appMessage(app, 'noteType', 'create')
      for (const key of ['download.s4.body_html', 'faq.q9.a_html']) {
        assert.ok(copy.get(key).includes(button), `${lang}: ${key} names the button “${button}”`)
        assert.ok(copy.get(key).includes(noteType), `${lang}: ${key} names the note type`)
      }
    }
  })

  test('German copy uses German quotes, English copy English ones', async () => {
    const [en, de] = await Promise.all([loadCopy('en'), loadCopy('de')])
    for (const [key, value] of de) {
      assert.ok(!/[“]/.test(value.replace(/„[^“]*“/g, '')), `de: ${key} has an unpaired “`)
    }
    for (const [key, value] of en) assert.ok(!value.includes('„'), `en: ${key} has a „`)
  })
})

describe('template rendering', () => {
  const render = (template, copy, flags = {}) =>
    renderTemplate(template, {
      lookup: (key) =>
        formatCopy(key, copy[key], { version: '1.2.3', url: 'https://a.b/?x=1&y=2' }),
      flags,
      partials: { dot: '<i>.</i>\n' },
      icons: { star: '<path d="M0 0"/>' },
    })

  test('escapes plain values and keeps _html values as markup', () => {
    const html = render('<p>{{a}}</p><p>{{b_html}}</p>', {
      a: 'Tom & <Jerry> "{version}"',
      b_html: '<a href="{url}">v{version}</a>',
    })
    assert.equal(
      html,
      '<p>Tom &amp; &lt;Jerry&gt; &quot;1.2.3&quot;</p>' +
        '<p><a href="https://a.b/?x=1&amp;y=2">v1.2.3</a></p>',
    )
  })

  test('keeps or drops sections by their flag, with standalone tag lines removed', () => {
    const template = 'a\n  {{#on}}\n  yes\n  {{/on}} {{^on}}\n  no\n  {{/on}}\nb'
    assert.equal(render(template, {}, { on: true }), 'a\n  yes\nb')
    assert.equal(render(template, {}, { on: false }), 'a\n  no\nb')
  })

  test('includes partials and icons', () => {
    const html = render('  {{>dot}}{{icon:star}}', {})
    assert.match(
      html,
      /^ {2}<i>\.<\/i><svg class="icon"[^>]*aria-hidden="true"[^>]*><path d="M0 0"\/><\/svg>$/,
    )
  })

  test('fails on unknown keys, tokens, flags, partials and icons', () => {
    assert.throws(() => render('{{#nope}}x{{/nope}}', {}), /Unknown section flag/)
    assert.throws(() => render('{{>nope}}', {}), /Unknown partial/)
    assert.throws(() => render('{{icon:nope}}', {}), /Unknown icon/)
    assert.throws(() => render('{{a}}', { a: 'v{nope}' }), /unknown token \{nope\}/)
    assert.throws(() => render('{{#on}}x', {}, { on: true }), /Unresolved template syntax/)
  })

  test('escapeHtml covers every special character', () => {
    assert.equal(escapeHtml(`&<>"'`), '&amp;&lt;&gt;&quot;&#39;')
  })
})

describe('helpers', () => {
  test('formats sizes in MB with one decimal, per language, kept on one line', () => {
    assert.equal(formatMegabytes(DMG_BYTES, 'en-US'), '2.5\u00a0MB')
    assert.equal(formatMegabytes(DMG_BYTES, 'de-DE'), '2,5\u00a0MB')
    assert.equal(formatMegabytes(12_340_000, 'de-DE'), '12,3\u00a0MB')
    assert.ok(!formatMegabytes(DMG_BYTES, 'en-US').includes(' '), 'no breakable space')
  })

  test('accepts only the universal Polywort disk image of the app version', () => {
    assert.equal(checkDmgName(DMG_NAME, VERSION), null)
    assert.equal(checkDmgName(DMG_NAME.toUpperCase(), VERSION), null)
    assert.throws(
      () => checkDmgName(`Polywort_${VERSION}_aarch64.dmg`, VERSION),
      /not the universal disk image/,
    )
    assert.throws(() => checkDmgName('Polywort.dmg', VERSION), /universal/)
    // An image built before the rename holds "Vocabulary Automator.app", not
    // the Polywort the install steps name, whatever its version.
    for (const other of [
      OLD_DMG_NAME,
      `x_${VERSION}_universal.dmg`,
      `Polywortx_${VERSION}_universal.dmg`,
    ]) {
      for (const allowVersionMismatch of [false, true]) {
        assert.throws(
          () => checkDmgName(other, VERSION, { allowVersionMismatch }),
          /is not a disk image of Polywort/,
          other,
        )
      }
    }
    // "1" + version contains the version, but it is another version.
    for (const other of ['Polywort_9.9.9_universal.dmg', `Polywort_1${VERSION}_universal.dmg`]) {
      assert.throws(() => checkDmgName(other, VERSION), /does not carry the app version/, other)
      assert.match(
        checkDmgName(other, VERSION, { allowVersionMismatch: true }),
        /^warning: .*does not carry the app version/,
      )
    }
  })

  test('publishes the disk image under a name without spaces', () => {
    assert.equal(publishedDmgName(DMG_NAME), DMG_PUBLISHED)
    assert.equal(publishedDmgName(OLD_DMG_NAME), `Vocabulary-Automator_${VERSION}_universal.dmg`)
    assert.equal(publishedDmgName('a b  (c).dmg'), 'a-b-c-.dmg')
  })

  test('normalizes SITE_URL and rejects what is not one', () => {
    assert.equal(normalizeSiteUrl(undefined), SITE_URL)
    assert.equal(normalizeSiteUrl('  '), SITE_URL)
    assert.equal(normalizeSiteUrl('https://example.org/'), 'https://example.org')
    assert.equal(normalizeSiteUrl('http://localhost:4173'), 'http://localhost:4173')
    // Every link in the page is root-relative, so the site cannot live under a path.
    assert.throws(() => normalizeSiteUrl('https://example.org/sub/'), /must be an origin/)
    assert.throws(() => normalizeSiteUrl('https://user@example.org'), /must be an origin/)
    assert.throws(() => normalizeSiteUrl('example.org'), /not a URL/)
    assert.throws(() => normalizeSiteUrl('ftp://example.org'), /http\(s\)/)
    assert.throws(() => normalizeSiteUrl('https://example.org/?a=1'), /query or hash/)
  })
})

/** Checks every built page must pass, with or without a disk image. */
function checkCommonOutput(label, getOut) {
  describe(`${label}: every page`, () => {
    let outDir
    let pages
    before(async () => {
      outDir = getOut()
      pages = await readPages(outDir)
    })

    test('has no template syntax left', () => {
      for (const [page, html] of Object.entries(pages)) {
        assert.ok(!html.includes('{{'), `${page} contains {{`)
        assert.ok(!html.includes('}}'), `${page} contains }}`)
        assert.ok(!/\{[a-z][A-Za-z0-9]*\}/.test(html), `${page} contains an unfilled {token}`)
      }
    })

    test('names the app Polywort in the title, the header and Open Graph', () => {
      for (const [page, html] of Object.entries(pages)) {
        assert.match(html, /<title>[^<]*Polywort[^<]*<\/title>/, `${page} title`)
        assert.ok(html.includes('<span class="brand-name">Polywort</span>'), `${page} wordmark`)
        assert.ok(!html.includes('Vocabulary Automator</'), `${page} shows the old name`)
      }
      for (const page of ['index.html', 'de/index.html']) {
        assert.ok(pages[page].includes('<meta property="og:site_name" content="Polywort" />'))
      }
    })

    test('declares its language', () => {
      assert.match(pages['index.html'], /^<!doctype html>\n<html lang="en">/)
      assert.match(pages['de/index.html'], /^<!doctype html>\n<html lang="de">/)
      assert.match(pages['404.html'], /^<!doctype html>\n<html lang="en">/)
    })

    test('has exactly one h1', () => {
      for (const [page, html] of Object.entries(pages)) {
        assert.equal(startTags(html, 'h1').length, 1, `${page} h1 count`)
      }
    })

    test('has an element for every in-page link and no duplicate ids', () => {
      for (const [page, html] of Object.entries(pages)) {
        const ids = [...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])
        assert.equal(new Set(ids).size, ids.length, `${page} has duplicate ids`)
        for (const [, target] of html.matchAll(/\shref="#([^"]*)"/g)) {
          assert.ok(ids.includes(target), `${page}: href="#${target}" has no matching id`)
        }
        for (const [, target] of html.matchAll(/\saria-(?:labelledby|describedby)="([^"]+)"/g)) {
          assert.ok(ids.includes(target), `${page}: aria reference ${target} has no matching id`)
        }
      }
    })

    test('links only to its own files and the allowed external pages', async () => {
      for (const [page, html] of Object.entries(pages)) {
        for (const [url] of html.matchAll(/https?:\/\/[^\s"'<>)]+/gi)) {
          const own = url === `${SITE_URL}/` || url.startsWith(`${SITE_URL}/`)
          assert.ok(own || ALLOWED_EXTERNAL.has(url), `${page} links to ${url}`)
        }
        // A protocol-relative URL ("//host/x") loads from another server too, in
        // any attribute: href, src, content, srcset, poster, action, data, …
        assert.ok(!/=\s*["']?\s*\/\//.test(html), `${page} has a protocol-relative URL`)
        for (const [, list] of html.matchAll(/\s(?:srcset|imagesrcset)\s*=\s*"([^"]*)"/gi)) {
          for (const candidate of list.split(',')) {
            const url = candidate.trim().split(/\s+/)[0]
            assert.ok(/^\/(?!\/)/.test(url), `${page} has a srcset entry from elsewhere: ${url}`)
          }
        }
        for (const [, local] of html.matchAll(/\s(?:href|src)="(\/[^"#]*)"/g)) {
          const file = local.endsWith('/') ? `${local}index.html` : local
          await stat(path.join(outDir, file)).catch(() => {
            assert.fail(`${page} refers to ${local}, which the build did not write`)
          })
        }
      }
    })

    test('loads nothing from other servers in the stylesheet and the script', async () => {
      const css = await readFile(path.join(outDir, 'styles.css'), 'utf8')
      assert.ok(!/@import/i.test(css), 'styles.css imports')
      for (const [, url] of css.matchAll(/url\(\s*['"]?([^'")]+)/g)) {
        assert.ok(!/^(?:https?:)?\/\//.test(url), `styles.css loads ${url}`)
      }
      const js = await readFile(path.join(outDir, 'main.js'), 'utf8')
      assert.ok(!/https?:\/\//.test(js), 'main.js names a URL')
      assert.ok(!/\b(?:fetch|XMLHttpRequest|sendBeacon|WebSocket|localStorage|cookie)\b/.test(js))
    })

    test('has no inline scripts, styles or event handlers', () => {
      for (const [page, html] of Object.entries(pages)) {
        for (const tag of startTags(html, 'script')) {
          assert.match(tag, /\ssrc="\/[^"]+"/, `${page} has an inline script: ${tag}`)
        }
        assert.equal(startTags(html, 'style').length, 0, `${page} has a <style> element`)
        assert.ok(!/<[^>]+\sstyle=/i.test(html), `${page} has a style attribute`)
        assert.ok(!/<[^>]+\son[a-z]+=/i.test(html), `${page} has an inline event handler`)
        assert.ok(!/javascript:/i.test(html), `${page} has a javascript: URL`)
      }
    })

    test('links the two languages to each other', () => {
      for (const page of ['index.html', 'de/index.html']) {
        const html = pages[page]
        assert.ok(html.includes(`<link rel="alternate" hreflang="en" href="${SITE_URL}/" />`))
        assert.ok(html.includes(`<link rel="alternate" hreflang="de" href="${SITE_URL}/de/" />`))
        assert.ok(
          html.includes(`<link rel="alternate" hreflang="x-default" href="${SITE_URL}/" />`),
        )
      }
      assert.ok(pages['index.html'].includes(`<link rel="canonical" href="${SITE_URL}/" />`))
      assert.ok(pages['de/index.html'].includes(`<link rel="canonical" href="${SITE_URL}/de/" />`))
      assert.match(pages['index.html'], /<a[^>]*href="\/de\/"[^>]*hreflang="de"/)
      assert.match(pages['de/index.html'], /<a[^>]*href="\/"[^>]*hreflang="en"/)
    })

    test('points Open Graph at its own preview image for each language', async () => {
      for (const [page, lang] of [
        ['index.html', 'en'],
        ['de/index.html', 'de'],
      ]) {
        const image = `${SITE_URL}/assets/og-${lang}.png`
        assert.ok(pages[page].includes(`<meta property="og:image" content="${image}" />`), page)
        assert.ok(
          pages[page].includes('<meta name="twitter:card" content="summary_large_image" />'),
        )
        await stat(path.join(outDir, 'assets', `og-${lang}.png`))
      }
    })

    test('writes the sitemap, robots.txt and the Vercel config', async () => {
      const sitemap = await readFile(path.join(outDir, 'sitemap.xml'), 'utf8')
      assert.ok(sitemap.includes(`<loc>${SITE_URL}/</loc>`))
      assert.ok(sitemap.includes(`<loc>${SITE_URL}/de/</loc>`))
      const robots = await readFile(path.join(outDir, 'robots.txt'), 'utf8')
      assert.ok(robots.includes(`Sitemap: ${SITE_URL}/sitemap.xml`))
      const ignored = (await readFile(path.join(outDir, '.vercelignore'), 'utf8')).split('\n')
      assert.ok(ignored.includes('.env*'), 'the token file `vercel link` writes is never uploaded')
      const vercel = JSON.parse(await readFile(path.join(outDir, 'vercel.json'), 'utf8'))
      assert.equal(vercel.cleanUrls, true)
      const headers = Object.fromEntries(
        vercel.headers.find((rule) => rule.source === '/(.*)').headers.map((h) => [h.key, h.value]),
      )
      assert.equal(
        headers['Content-Security-Policy'],
        "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; " +
          "font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; " +
          "form-action 'none'; frame-ancestors 'none'",
      )
      assert.equal(headers['X-Content-Type-Options'], 'nosniff')
      assert.equal(headers['Referrer-Policy'], 'strict-origin-when-cross-origin')
      assert.match(headers['Permissions-Policy'], /camera=\(\).*microphone=\(\).*geolocation=\(\)/)
    })

    test('applies the Vercel header rules to the URLs the build publishes', async () => {
      const vercel = JSON.parse(await readFile(path.join(outDir, 'vercel.json'), 'utf8'))
      /** The headers Vercel sends for `url`, read with the preview server's pattern logic. */
      const headersFor = (url) =>
        Object.fromEntries(
          vercel.headers
            .filter((rule) => sourcePattern(rule.source).test(url))
            .flatMap((rule) => rule.headers.map((h) => [h.key, h.value])),
        )
      for (const url of ['/', '/de/', '/404.html', '/styles.css', '/main.js']) {
        assert.match(headersFor(url)['Content-Security-Policy'] ?? '', /default-src 'self'/, url)
      }
      const dmg = headersFor(`/downloads/${DMG_PUBLISHED}`)
      assert.equal(dmg['Content-Type'], 'application/x-apple-diskimage')
      assert.equal(dmg['Content-Disposition'], 'attachment')
      assert.ok(dmg['Content-Security-Policy'], 'the disk image gets the security headers too')
      assert.equal(headersFor('/')['Content-Type'], undefined, 'pages keep their own type')
    })
  })
}

describe('build with a disk image', () => {
  let outDir
  let sha256
  let result

  before(async () => {
    const downloads = await tempDir('downloads')
    // An older image that must lose against the newest one.
    const older = 'Polywort_0.0.0_universal.dmg'
    await fakeDmg(downloads, older, 1_000, 1)
    const old = new Date('2026-01-01T00:00:00Z')
    await utimes(path.join(downloads, older), old, old)
    sha256 = await fakeDmg(downloads, DMG_NAME, DMG_BYTES)
    await writeFile(path.join(downloads, 'notes.txt'), 'not a disk image')
    outDir = path.join(await tempDir('out'), 'dist')
    result = await build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL })
  })

  test('copies the newest .dmg under its published name', async () => {
    assert.equal(result.dmg.file, DMG_PUBLISHED)
    assert.deepEqual(await readdir(path.join(outDir, 'downloads')), [DMG_PUBLISHED])
    const copy = await readFile(path.join(outDir, 'downloads', DMG_PUBLISHED))
    assert.equal(createHash('sha256').update(copy).digest('hex'), sha256)
  })

  test('renders the download link, size and checksum in both languages', async () => {
    const pages = await readPages(outDir)
    for (const [page, size] of [
      ['index.html', '2.5\u00a0MB'],
      ['de/index.html', '2,5\u00a0MB'],
    ]) {
      const html = pages[page]
      assert.ok(html.includes(`href="/downloads/${DMG_PUBLISHED}"`), `${page} download link`)
      // The separator and the size stay on the line before them.
      assert.ok(html.includes(`&nbsp;· ${size}</p>`), `${page} size ${size}`)
      assert.ok(html.includes(`<code id="sha256" class="checksum-value">${sha256}</code>`))
      assert.ok(html.includes(`shasum -a 256 ~/Downloads/${DMG_PUBLISHED}`), `${page} shasum`)
      assert.ok(html.includes('data-copy="sha256"'), `${page} copy button`)
      assert.match(
        html,
        /<button[^>]*data-copy="sha256"[^>]*\shidden/,
        'the copy button waits for JS',
      )
      const primary = html.match(/<a class="btn btn-primary btn-lg" href="([^"]+)"/g)
      assert.equal(primary.length, 2, `${page} has a hero and a download-section button`)
      for (const tag of primary) assert.ok(tag.includes(`href="/downloads/${DMG_PUBLISHED}"`), tag)
    }
    assert.ok(
      pages['index.html'].includes(
        `Version\u00a0${VERSION}\u00a0· Preview\u00a0· Apple Silicon and Intel`,
      ),
    )
  })

  checkCommonOutput('with a disk image', () => outDir)
})

describe('build without a disk image', () => {
  let outDir
  let result

  before(async () => {
    outDir = path.join(await tempDir('out'), 'dist')
    const missing = path.join(await tempDir('none'), 'does-not-exist')
    result = await build({ outDir, downloadsDir: missing, siteUrl: `${SITE_URL}/` })
  })

  test('links the macOS buttons to GitHub Releases and says the download is not available yet', async () => {
    assert.equal(result.dmg, null)
    await assert.rejects(stat(path.join(outDir, 'downloads')), { code: 'ENOENT' })
    const pages = await readPages(outDir)
    for (const [page, notYet] of [
      ['index.html', 'The macOS download is not available yet.'],
      ['de/index.html', 'Der Download für macOS ist noch nicht verfügbar.'],
    ]) {
      const html = pages[page]
      assert.ok(!html.includes('/downloads/'), `${page} links to a download`)
      assert.ok(!html.includes('id="sha256"'), `${page} shows a checksum`)
      assert.ok(html.includes(notYet), `${page} says the download is not available yet`)
      const primary = html.match(/<a class="btn btn-primary btn-lg" href="([^"]+)"/g)
      assert.equal(primary.length, 2, `${page} has a hero and a download-section button`)
      for (const tag of primary) assert.ok(tag.includes(`href="${RELEASES_URL}"`), tag)
    }
  })

  checkCommonOutput('without a disk image', () => outDir)
})

describe('build options', () => {
  test('SITE_URL sets canonical URLs, Open Graph URLs and the sitemap', async () => {
    const outDir = path.join(await tempDir('out'), 'dist')
    const downloads = await tempDir('downloads')
    await build({ outDir, downloadsDir: downloads, siteUrl: 'https://example.org/' })
    const html = await readFile(path.join(outDir, 'de', 'index.html'), 'utf8')
    assert.ok(html.includes('<link rel="canonical" href="https://example.org/de/" />'))
    assert.ok(html.includes('<meta property="og:url" content="https://example.org/de/" />'))
    assert.ok(html.includes('content="https://example.org/assets/og-de.png"'))
    assert.ok(!html.includes(SITE_URL))
    const sitemap = await readFile(path.join(outDir, 'sitemap.xml'), 'utf8')
    assert.ok(sitemap.includes('<loc>https://example.org/de/</loc>'))
  })

  test('refuses to empty an output folder that holds something else', async () => {
    const outDir = await tempDir('precious')
    await writeFile(path.join(outDir, 'thesis.txt'), 'do not delete')
    await assert.rejects(
      build({ outDir, downloadsDir: await tempDir('downloads'), siteUrl: SITE_URL }),
      /Refusing to empty/,
    )
    assert.equal(await readFile(path.join(outDir, 'thesis.txt'), 'utf8'), 'do not delete')
  })

  test('a failed build leaves the output folder as it was, and the next build succeeds', async () => {
    const broken = await tempDir('src')
    await cp(path.join(SITE_DIR, 'src'), broken, { recursive: true })
    const template = await readFile(path.join(broken, 'page.html'), 'utf8')
    await writeFile(
      path.join(broken, 'page.html'),
      template.replace('{{nav.skip}}', '{{nav.skipp}}'),
    )
    const downloads = await tempDir('downloads')

    // A first build that fails creates nothing.
    const outDir = path.join(await tempDir('out'), 'dist')
    await assert.rejects(
      build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL, srcDir: broken }),
      /Unknown copy key \{\{nav\.skipp\}\}/,
    )
    await assert.rejects(stat(outDir), { code: 'ENOENT' })

    // A build that fails over an earlier one keeps it, and a fixed build replaces it.
    await build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL })
    const before = await readdir(outDir)
    await assert.rejects(
      build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL, srcDir: broken }),
      /Unknown copy key/,
    )
    assert.deepEqual(await readdir(outDir), before)
    await build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL })
    assert.ok((await readFile(path.join(outDir, 'index.html'), 'utf8')).includes('id="main"'))
  })

  test('refuses a disk image built before the rename, and leaves the output folder alone', async () => {
    const downloads = await tempDir('downloads')
    await fakeDmg(downloads, OLD_DMG_NAME, 10)
    const outDir = path.join(await tempDir('out'), 'dist')
    for (const allowVersionMismatch of [false, true]) {
      await assert.rejects(
        build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL, allowVersionMismatch }),
        /is not a disk image of Polywort/,
      )
    }
    await assert.rejects(stat(outDir), { code: 'ENOENT' })
  })

  test('refuses a disk image of another version unless told to publish it', async () => {
    const downloads = await tempDir('downloads')
    await fakeDmg(downloads, 'Polywort_9.9.9_universal.dmg', 10)
    const outDir = path.join(await tempDir('out'), 'dist')
    await assert.rejects(
      build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL }),
      /does not carry the app version/,
    )
    await assert.rejects(stat(outDir), { code: 'ENOENT' })
    const result = await build({
      outDir,
      downloadsDir: downloads,
      siteUrl: SITE_URL,
      allowVersionMismatch: true,
    })
    assert.equal(result.warnings.length, 1)
    assert.equal(result.dmg.file, 'Polywort_9.9.9_universal.dmg')
  })

  test('builds when the script is run through a symlink', async (t) => {
    const dir = await tempDir('link')
    const link = path.join(dir, 'build.mjs')
    try {
      await symlink(path.join(SITE_DIR, 'build.mjs'), link)
    } catch (error) {
      t.skip(`cannot create a symlink here: ${error.code}`)
      return
    }
    const outDir = path.join(dir, 'out')
    await promisify(execFile)(
      process.execPath,
      [link, '--out', outDir, '--downloads', path.join(dir, 'none')],
      { env: { ...process.env, SITE_URL } },
    )
    await stat(path.join(outDir, 'index.html'))
  })

  test('rebuilds over an earlier build and keeps the Vercel link', async () => {
    const outDir = path.join(await tempDir('out'), 'dist')
    const downloads = await tempDir('downloads')
    await fakeDmg(downloads, DMG_NAME, 10)
    await build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL })
    await mkdir(path.join(outDir, '.vercel'))
    await writeFile(path.join(outDir, '.vercel', 'project.json'), '{}')
    await rm(path.join(downloads, DMG_NAME))
    await build({ outDir, downloadsDir: downloads, siteUrl: SITE_URL })
    await assert.rejects(stat(path.join(outDir, 'downloads')), { code: 'ENOENT' })
    assert.equal(await readFile(path.join(outDir, '.vercel', 'project.json'), 'utf8'), '{}')
  })
})
