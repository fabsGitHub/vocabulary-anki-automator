import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Learn's frontend. `--mode web` builds the standalone browser version
// (decision 10), which swaps the Tauri platform adapter for the web one.
const root = fileURLToPath(new URL('.', import.meta.url))

export default defineConfig(({ mode }) => ({
  root,
  plugins: [vue()],
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
