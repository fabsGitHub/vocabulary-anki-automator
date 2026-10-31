import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'

// Learn's frontend. `--mode web` builds the standalone browser version
// (decision 10), which swaps the Tauri platform adapter for the web one.
// `--mode demo` builds that browser version as the public live demo
// (docs/live-demo.md): the same app, entered through src/demo/main.ts, which
// starts it with a made-up history in a database of its own.
const root = fileURLToPath(new URL('.', import.meta.url))

// The desktop build drops the web adapter's code (src/platform/index.ts), but
// Vite still loads its modules and emits the assets they import, used or
// not: the 2 MB curriculum JSON, which the desktop app already compiles into
// its binary. Outside the web build that import is an empty URL instead.
const withoutWebCurriculum: Plugin = {
  name: 'learn:without-web-curriculum',
  apply: 'build',
  enforce: 'pre',
  load(id) {
    return id.endsWith('/hsk_curriculum.json?url') ? 'export default ""' : null
  },
}

const APP_ENTRY = '/src/main.ts'
const DEMO_ENTRY = '/src/demo/main.ts'
const DEMO_DESCRIPTION =
  'Live demo of Vocabulary Learn: spaced-repetition reviews, exercises and lessons, running entirely in your browser.'

// The demo's page loads the demo's entry instead of the app's, so nothing of
// the demo reaches the other builds, and describes itself for link previews.
const demoEntry: Plugin = {
  name: 'learn:demo-entry',
  transformIndexHtml: {
    order: 'pre',
    handler(html) {
      if (!html.includes(APP_ENTRY)) throw new Error(`index.html no longer loads ${APP_ENTRY}`)
      const meta = [
        `<meta name="description" content="${DEMO_DESCRIPTION}" />`,
        '<meta property="og:title" content="Vocabulary Learn – live demo" />',
        `<meta property="og:description" content="${DEMO_DESCRIPTION}" />`,
      ]
      return html
        .replace(APP_ENTRY, DEMO_ENTRY)
        .replace('</head>', `  ${meta.join('\n    ')}\n  </head>`)
    },
  },
}

export default defineConfig(({ mode }) => {
  const web = mode === 'web' || mode === 'demo'
  return {
    root,
    plugins: mode === 'demo' ? [vue(), demoEntry] : web ? [vue()] : [vue(), withoutWebCurriculum],
    clearScreen: false,
    base: './',
    define: {
      __LEARN_PLATFORM__: JSON.stringify(web ? 'web' : 'tauri'),
    },
    build: {
      outDir: mode === 'demo' ? 'dist-demo' : web ? 'dist-web' : 'dist',
      emptyOutDir: true,
    },
    server: {
      port: 1430,
      strictPort: true,
      watch: { ignored: ['**/src-tauri/**'] },
    },
  }
})
