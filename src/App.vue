<script setup lang="ts">
/**
 * Both windows load this one bundle (docs/architecture.md § 8.3); the window
 * label decides which UI it is. The label never changes for a webview, so it
 * is read once.
 */
import { currentWindowLabel } from './api'
import ApprovalWindow from './views/ApprovalWindow.vue'
import MainWindow from './views/MainWindow.vue'

const isPreview = currentWindowLabel() === 'preview'
</script>

<template>
  <ApprovalWindow v-if="isPreview" />
  <MainWindow v-else />
</template>
