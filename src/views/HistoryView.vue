<script setup lang="ts">
/**
 * The "History" tab (Milestone F): the latest captures with their Anki sync
 * state, deleting with an inline confirmation (no native dialog, which a
 * webview may not provide), and pushing the captures Anki never received.
 */
import { computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'

import type { CaptureOrigin, StoredCapture } from '../api'
import { HISTORY_LIMIT, useHistory } from '../composables/useHistory'

const { t, te, locale } = useI18n()
const history = useHistory()
const { items, loading, loadError, confirmingId, deletingId, syncing, syncReport, notice } = history

/**
 * What "Sync pending to Anki" will push: `sync_pending_captures` takes
 * `origin = capture` rows only (docs/architecture.md § 8.2). The words Learn
 * added to the shared history (curriculum words, Anki imports) are listed
 * too, but Learn's own export sends them, so counting them here would show
 * a number that syncing never brings down.
 */
const pendingCount = computed(
  () => items.value.filter((item) => item.origin === 'capture' && !item.synced_to_anki).length,
)

function formatDate(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString(locale.value, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function originLabel(origin: CaptureOrigin): string {
  const key = `history.origin.${origin}`
  return te(key) ? t(key) : origin
}

function pinyinOf(item: StoredCapture): string | null {
  const metadata = item.card.script_metadata
  return metadata.kind === 'chinese' && metadata.pinyin ? metadata.pinyin : null
}

onMounted(() => void history.start())
</script>

<template>
  <section class="pane-layout history-pane">
    <div class="pane-title-row">
      <div>
        <h3 class="pane-title">{{ t('history.title') }}</h3>
        <p class="input-hint">{{ t('history.limitNote', { limit: HISTORY_LIMIT }) }}</p>
      </div>
      <div class="toolbar">
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="refresh"
          :disabled="loading"
          @click="history.refresh()"
        >
          {{ t('history.refresh') }}
        </button>
        <button
          type="button"
          class="btn btn-primary btn-mini"
          data-test="sync"
          :disabled="syncing"
          @click="history.syncPending()"
        >
          {{ syncing ? t('history.syncing') : t('history.sync') }}
          <span v-if="pendingCount > 0 && !syncing" class="count-pill">{{ pendingCount }}</span>
        </button>
      </div>
    </div>

    <div
      v-if="syncReport"
      class="notice"
      :class="syncReport.failed.length > 0 ? 'warning' : 'success'"
      data-test="sync-report"
    >
      <p v-if="syncReport.attempted === 0">{{ t('history.syncNothing') }}</p>
      <p v-else>
        {{
          t('history.syncResult', { synced: syncReport.synced, attempted: syncReport.attempted })
        }}
      </p>
      <template v-if="syncReport.failed.length > 0">
        <p>{{ t('history.syncFailures') }}</p>
        <ul class="notice-list">
          <li v-for="failure in syncReport.failed" :key="failure.id">
            {{
              t('history.syncFailure', {
                front: syncReport.failedFronts[failure.id],
                error: failure.error,
              })
            }}
          </li>
        </ul>
      </template>
    </div>
    <p v-if="notice" :class="['inline-result', notice.tone]" data-test="history-notice">
      {{ t(notice.key, notice.params ?? {}) }}
    </p>
    <p v-if="loadError" class="notice error" data-test="history-error">
      {{ t('history.loadFailed', { error: loadError }) }}
    </p>

    <p v-if="loading && items.length === 0" class="input-hint">{{ t('history.loading') }}</p>
    <p v-else-if="!loadError && items.length === 0" class="empty-state" data-test="history-empty">
      {{ t('history.empty') }}
    </p>

    <ul v-else class="history-list">
      <li
        v-for="item in items"
        :key="item.id"
        class="history-item"
        data-test="history-item"
        :data-id="item.id"
      >
        <div class="history-main">
          <div class="history-words">
            <span class="history-front" data-test="front">{{ item.card.front }}</span>
            <span class="history-arrow" aria-hidden="true">→</span>
            <span class="history-back" data-test="back">{{ item.card.back }}</span>
          </div>
          <div v-if="pinyinOf(item)" class="history-pinyin">{{ pinyinOf(item) }}</div>
          <div class="history-meta">
            <time :datetime="item.card.captured_at" data-test="date">{{
              formatDate(item.card.captured_at)
            }}</time>
            <span class="badge badge-muted" data-test="origin">{{ originLabel(item.origin) }}</span>
            <span
              :class="['badge', item.synced_to_anki ? 'badge-success' : 'badge-warning']"
              data-test="synced"
            >
              {{ item.synced_to_anki ? t('history.synced') : t('history.pending') }}
            </span>
          </div>
        </div>

        <div class="history-actions">
          <template v-if="confirmingId === item.id">
            <span class="inline-confirm" data-test="confirm-text">
              {{ t('history.confirmDelete', { front: item.card.front }) }}
            </span>
            <button
              type="button"
              class="btn btn-danger btn-mini"
              data-test="confirm-delete"
              :disabled="deletingId !== null"
              @click="history.confirmDelete(item.id)"
            >
              {{ t('history.confirmYes') }}
            </button>
            <button
              type="button"
              class="btn btn-secondary btn-mini"
              data-test="cancel-delete"
              @click="history.cancelDelete()"
            >
              {{ t('history.confirmNo') }}
            </button>
          </template>
          <button
            v-else
            type="button"
            class="btn btn-secondary btn-mini"
            data-test="delete"
            :disabled="deletingId !== null"
            @click="history.askDelete(item.id)"
          >
            {{ t('history.delete') }}
          </button>
        </div>
      </li>
    </ul>
  </section>
</template>
