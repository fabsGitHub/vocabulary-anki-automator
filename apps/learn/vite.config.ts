import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'

// Learn's frontend. `--mode web` builds the standalone browser version
// (decision 10), which swaps the Tauri platform adapter for the web one.
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

export default defineConfig(({ mode }) => ({
  root,
  plugins: mode === 'web' ? [vue()] : [vue(), withoutWebCurriculum],
  clearScreen: false,
  base: './',
  define: {
    __LEARN_PLATFORM__: JSON.stringify(mode === 'web' ? 'web' : 'tauri'),
  },
  build: {
    outDir: mode === 'web' ? 'dist-web' : 'dist',
    emptyOutDir: true,
  },
  server: {
    port: 1430,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
}))
