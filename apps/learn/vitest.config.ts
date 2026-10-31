import { fileURLToPath } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

// Learn's unit tests (`npx vitest run --config apps/learn/vitest.config.ts`).
// Rooted at apps/learn like vite.config.ts, so `src/**` below is
// apps/learn/src/** wherever the command runs from. The tests exercise the
// desktop build's code paths (`__LEARN_PLATFORM__` = 'tauri'); views get a
// fake PlatformAdapter, and the Tauri adapter's test mocks `invoke`.
const root = fileURLToPath(new URL('.', import.meta.url))

export default defineConfig({
  root,
  plugins: [vue()],
  define: {
    __LEARN_PLATFORM__: JSON.stringify('tauri'),
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test/setup.ts'],
    globals: false,
  },
})
