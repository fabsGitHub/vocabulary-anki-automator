<script setup lang="ts">
/** The summary of an Anki export or import (`AnkiSyncReport`), failures listed one by one. */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { AnkiSyncReport } from '../platform/types'

const props = defineProps<{ report: AnkiSyncReport; mode: 'export' | 'import' }>()

const { t } = useI18n()

const tone = computed(() => (props.report.failed.length > 0 ? 'warning' : 'success'))
const summary = computed(() => {
  const { attempted, succeeded } = props.report
  if (props.mode === 'export') {
    return attempted === 0
      ? t('anki.exportNothing')
      : t('anki.exportResult', { succeeded, attempted })
  }
  return attempted === 0
    ? t('anki.importNothing')
    : t('anki.importResult', { succeeded, attempted })
})
const skipped = computed(() => {
  const count = props.report.skipped
  if (count === 0) return null
  return props.mode === 'export'
    ? t('anki.exportSkipped', { skipped: count })
    : t('anki.importSkipped', { skipped: count })
})

function failedFront(failure: AnkiSyncReport['failed'][number]): string {
  if (failure.front.trim()) return failure.front
  return failure.card_id === null ? '—' : t('anki.unnamedCard', { id: failure.card_id })
}
</script>

<template>
  <div class="notice" :class="tone" :data-test="`${mode}-report`">
    <p data-test="report-summary">{{ summary }}</p>
    <p v-if="skipped" data-test="report-skipped">{{ skipped }}</p>
    <template v-if="report.failed.length > 0">
      <p data-test="report-failures">{{ t('anki.failures', { count: report.failed.length }) }}</p>
      <ul class="notice-list">
        <li v-for="(failure, index) in report.failed" :key="index" data-test="report-failure">
          {{ t('anki.failure', { front: failedFront(failure), error: failure.error }) }}
        </li>
      </ul>
    </template>
  </div>
</template>
