#!/usr/bin/env node
// A small static server for previewing the built site (site/dist) locally,
// with the headers from site/vercel.json (the Content-Security-Policy among
// them), clean URLs and the 404 page, so the preview behaves like the Vercel
// deployment. Node built-ins only.
//
//   node site/serve.mjs [--dir <dir>] [--port <port>]   (default site/dist, 4173)

import { createReadStream } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import { createServer } from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const SITE_DIR = path.dirname(fileURLToPath(import.meta.url))

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.xml': 'application/xml; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.dmg': 'application/octet-stream',
}

/** A vercel.json `source` such as `/downloads/(.*\.dmg)` as a RegExp: groups are regex, the rest literal. */
function sourcePattern(source) {
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

async function isFile(file) {
  try {
    return (await stat(file)).isFile()
  } catch {
    return false
  }
}

/** The file a clean URL maps to, or null: `/` → index.html, `/de/` → de/index.html, `/x` → x.html. */
async function resolveFile(root, pathname) {
  const target = path.resolve(root, `.${pathname}`)
  if (target !== root && !target.startsWith(root + path.sep)) return null
  for (const candidate of [target, path.join(target, 'index.html'), `${target}.html`]) {
    if (await isFile(candidate)) return candidate
  }
  return null
}

async function main() {
  const { values } = parseArgs({
    options: { dir: { type: 'string' }, port: { type: 'string' } },
  })
  const root = path.resolve(values.dir ?? path.join(SITE_DIR, 'dist'))
  const port = Number(values.port ?? process.env.PORT ?? 4173)
  const config = JSON.parse(await readFile(path.join(root, 'vercel.json'), 'utf8'))
  const rules = config.headers.map((rule) => ({
    pattern: sourcePattern(rule.source),
    headers: rule.headers,
  }))

  const server = createServer(async (request, response) => {
    const { pathname } = new URL(request.url, 'http://localhost')
    let decoded
    try {
      decoded = decodeURIComponent(pathname)
    } catch {
      decoded = '/'
    }
    for (const rule of rules) {
      if (rule.pattern.test(decoded)) {
        for (const { key, value } of rule.headers) response.setHeader(key, value)
      }
    }
    let file = await resolveFile(root, decoded)
    let status = 200
    if (!file) {
      file = path.join(root, '404.html')
      status = 404
    }
    if (!response.hasHeader('Content-Type')) {
      response.setHeader('Content-Type', TYPES[path.extname(file)] ?? 'application/octet-stream')
    }
    response.setHeader('Cache-Control', 'no-cache')
    response.writeHead(status)
    if (request.method === 'HEAD') {
      response.end()
      return
    }
    createReadStream(file).pipe(response)
  })

  server.listen(port, '127.0.0.1', () => {
    console.log(`Serving ${path.relative(process.cwd(), root) || '.'} at http://localhost:${port}/`)
  })
}

main().catch((error) => {
  console.error(error.message)
  process.exit(1)
})
