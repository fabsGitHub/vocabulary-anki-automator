<script setup lang="ts">
/**
 * The non-blocking "update available" banner (Milestone N4); mounted in the
 * main window only, so the approval window never checks for updates. The
 * timing and failure rules live in `useUpdater`.
 */
import { onMounted } from 'vue'
import { useI18n } from 'vue-i18n'

import { useUpdater, type UpdaterOptions } from '../composables/useUpdater'

const props = defineProps<{ options?: UpdaterOptions }>()

const { t } = useI18n()
const updater = useUpdater(props.options)
const { update, version, installing, failed, blockedByCapture } = updater

onMounted(() => updater.start())
</script>

<template>
  <div v-if="update" class="update-banner" role="status" data-test="update-banner">
    <span class="update-text">{{ t('update.available', { version }) }}</span>
    <span v-if="blockedByCapture" class="update-note">{{ t('update.busy') }}</span>
    <span v-else-if="failed" class="update-note">{{ t('update.failed') }}</span>
    <button
      type="button"
      class="btn btn-primary btn-mini"
      data-test="update-install"
      :disabled="installing"
      @click="updater.install()"
    >
      {{ installing ? t('update.installing') : t('update.install') }}
    </button>
    <button
      type="button"
      class="close-btn"
      data-test="update-dismiss"
      :aria-label="t('update.dismiss')"
      :title="t('update.dismiss')"
      :disabled="installing"
      @click="updater.dismiss()"
    >
      &times;
    </button>
  </div>
</template>
