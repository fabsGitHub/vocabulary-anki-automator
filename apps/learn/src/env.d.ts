/// <reference types="vite/client" />

declare const __LEARN_PLATFORM__: 'tauri' | 'web'

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<object, object, unknown>
  export default component
}
