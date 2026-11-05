<script setup lang="ts">
/**
 * The live demo's root (docs/live-demo.md): Learn's window with the demo's
 * bar in its banner slot, mounted afresh whenever the demo's clock moves or
 * the demo starts over, so that every view reloads from the store.
 */
import App from '../App.vue'
import DemoBar from './DemoBar.vue'
import { useDemo } from './demo'

const { epoch } = useDemo()
</script>

<template>
  <App :key="epoch">
    <template #banner>
      <DemoBar />
    </template>
  </App>
</template>
