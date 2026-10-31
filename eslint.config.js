// ESLint flat config for the Vue 3 + TypeScript frontend (src/).
// No lint config existed before Milestone A — this is intentionally minimal:
// Vue's recommended rules + TS parsing + Prettier compatibility, nothing
// project-specific yet.
import js from '@eslint/js'
import vue from 'eslint-plugin-vue'
import tsParser from '@typescript-eslint/parser'
import tsPlugin from '@typescript-eslint/eslint-plugin'
import vueParser from 'vue-eslint-parser'
import prettierConfig from '@vue/eslint-config-prettier'

export default [
  js.configs.recommended,
  ...vue.configs['flat/recommended'],
  {
    files: ['**/*.ts', '**/*.vue'],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        parser: tsParser,
        sourceType: 'module',
        ecmaVersion: 'latest',
        extraFileExtensions: ['.vue'],
      },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
    },
    rules: {
      'vue/multi-word-component-names': 'off',
      // The core rules don't understand TypeScript (type-only declarations,
      // ambient globals); vue-tsc's strict mode already catches undefined
      // names, and the TS-aware variant handles unused ones.
      'no-undef': 'off',
      'no-unused-vars': 'off',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrors: 'none' },
      ],
    },
  },
  {
    // Ambient declaration files only declare; nothing in them is "used".
    files: ['**/*.d.ts'],
    rules: {
      '@typescript-eslint/no-unused-vars': 'off',
    },
  },
  {
    // *.config.ts / *.config.js files (vite.config.ts, vitest.config.ts,
    // apps/learn/vite.config.ts) run under Node, not the browser — they
    // reference `process` (e.g. `process.env`) which the base config above
    // doesn't know about.
    files: ['**/*.config.ts', '**/*.config.js'],
    languageOptions: {
      globals: {
        process: 'readonly',
      },
    },
  },
  {
    // Node scripts (scripts/release-preflight.mjs) run in CI, not the browser.
    files: ['scripts/**/*.mjs'],
    languageOptions: {
      globals: {
        Buffer: 'readonly',
        console: 'readonly',
        process: 'readonly',
        URL: 'readonly',
      },
    },
  },
  prettierConfig,
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'target/**',
      'src-tauri/target/**',
      'src-tauri/gen/**',
      'crates/**/target/**',
      'apps/**/dist/**',
      'apps/**/dist-web/**',
      'apps/**/src-tauri/gen/**',
      '.claude/**',
    ],
  },
]
