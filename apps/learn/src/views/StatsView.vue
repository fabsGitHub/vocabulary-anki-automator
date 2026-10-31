<script setup lang="ts">
/**
 * XP, streak and learning counts (Milestone I), as `getStats` reports them.
 * The backend spends due streak freezes before it answers, so the numbers
 * are current; they are fetched again every time the tab is shown, since a
 * review or an exercise in between changes them.
 */
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePanelActive } from '../composables/useShortcuts'
import { usePlatform } from '../context'
import { errorMessage } from '../domain/errors'
import type { StatsSummary } from '../platform/types'

const { t, n } = useI18n()
const platform = usePlatform()
const active = usePanelActive()

const stats = ref<StatsSummary | null>(null)
const loading = ref(false)
const loadError = ref<string | null>(null)

async function load(): Promise<void> {
  if (loading.value) return
  loading.value = true
  loadError.value = null
  try {
    stats.value = await platform.getStats()
  } catch (err) {
    loadError.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

watch(
  active,
  (isActive) => {
    if (isActive) void load()
  },
  { immediate: true },
)
</script>

<template>
  <section class="pane-layout stats-pane" data-test="stats">
    <div class="pane-title-row">
      <h2 class="pane-title">{{ t('stats.title') }}</h2>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="refresh"
        :disabled="loading"
        @click="load"
      >
        {{ t('stats.refresh') }}
      </button>
    </div>

    <p v-if="loadError" class="notice error" role="alert" data-test="stats-error">
      {{ t('stats.loadFailed', { error: loadError }) }}
    </p>
    <p v-if="!stats && loading" class="input-hint spinner">{{ t('stats.loading') }}</p>

    <template v-if="stats">
      <p :class="stats.streak.completed_today ? 'success-notice-card' : 'notice'" data-test="today">
        {{ stats.streak.completed_today ? t('stats.todayDone') : t('stats.todayOpen') }}
      </p>
      <dl class="stat-grid">
        <div class="stat-tile">
          <dt>{{ t('stats.xpTotal') }}</dt>
          <dd data-test="xp-total">{{ n(stats.xp_total) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.xpToday') }}</dt>
          <dd data-test="xp-today">{{ n(stats.xp_today) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.streakCurrent') }}</dt>
          <dd data-test="streak-current">{{ n(stats.streak.current) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.streakLongest') }}</dt>
          <dd data-test="streak-longest">{{ n(stats.streak.longest) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.freezes') }}</dt>
          <dd data-test="freezes">{{ n(stats.streak.freezes_available) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.learned') }}</dt>
          <dd data-test="learned">{{ n(stats.learned_count) }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('stats.dueToday') }}</dt>
          <dd data-test="due-today">{{ n(stats.due_today) }}</dd>
        </div>
      </dl>
      <p class="input-hint">{{ t('stats.freezesHint') }}</p>
    </template>
  </section>
</template>
