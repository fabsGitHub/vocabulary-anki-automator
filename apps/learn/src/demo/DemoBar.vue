<script setup lang="ts">
/**
 * The live demo's bar above Learn's header (docs/live-demo.md): what the
 * demo is, its date once "Next day" has moved the clock, the two actions
 * and, folded away, what the project is. It sits in the app's banner slot,
 * so the daily prompt makes it inert along with the rest of the window.
 *
 * `VITE_DEMO_PROJECT_URL`, when the demo is built with it, adds a link to
 * the page that presents the project.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useDemo } from './demo'
import { demoMessages } from './messages'

const { t, locale } = useI18n({ messages: demoMessages })
const demo = useDemo()
const { dayShift, busy, error, persistent } = demo
const projectUrl: string | undefined = import.meta.env.VITE_DEMO_PROJECT_URL || undefined

// The clock is not reactive, but the bar is mounted afresh with the app
// whenever the clock moves.
const date = computed(() =>
  new Intl.DateTimeFormat(locale.value, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
  }).format(demo.now()),
)
</script>

<template>
  <aside class="demo-bar" :aria-label="t('label')" data-test="demo-bar">
    <div class="demo-bar-row">
      <span class="badge">{{ t('label') }}</span>
      <p class="demo-bar-text" data-test="demo-summary">
        {{ t(persistent ? 'summary' : 'summaryMemory') }}
      </p>
      <span v-if="dayShift > 0" class="badge badge-muted" data-test="demo-date">
        {{ t('date', { date }) }}
      </span>
      <div class="demo-bar-actions">
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="demo-next-day"
          :title="t('nextDayHint')"
          :aria-disabled="busy || undefined"
          @click="demo.nextDay()"
        >
          {{ t('nextDay') }}
        </button>
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="demo-start-over"
          :title="t('startOverHint')"
          :aria-disabled="busy || undefined"
          @click="demo.reset()"
        >
          {{ t('startOver') }}
        </button>
      </div>
    </div>
    <p v-if="error" class="notice error" role="alert" data-test="demo-error">
      {{ t('failed', { error }) }}
    </p>
    <details class="demo-about">
      <summary>{{ t('about') }}</summary>
      <p>{{ t('aboutApps') }}</p>
      <p>{{ t('aboutTech') }}</p>
      <p v-if="projectUrl">
        <a :href="projectUrl" target="_blank" rel="noopener" data-test="demo-project-link">
          {{ t('projectLink') }}
        </a>
      </p>
    </details>
  </aside>
</template>
