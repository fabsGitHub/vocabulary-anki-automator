<script setup lang="ts">
/**
 * The consent dialog shown before either install action runs (Milestone E,
 * docs/architecture.md § 8.4). It states what is downloaded, from where, how
 * it is verified and what ends up where, and only emits: recording the
 * consent and starting the download stay with the caller, so "Cancel" can
 * never have started anything.
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { ConsentAction } from '../api'
import {
  ANKICONNECT_ADDON_DIRS,
  ANKICONNECT_COMMIT,
  ANKICONNECT_FILES,
  ANKICONNECT_REPOSITORY,
  ANKI_INSTALLERS,
  ANKI_PINNED_VERSION,
  ankiReleaseUrl,
} from '../domain/supplyChain'

const props = defineProps<{
  action: ConsentAction
  /** The Anki version `get_setup_status` reports. */
  ankiVersion: string
}>()

const emit = defineEmits<{ agree: []; cancel: [] }>()

const { t } = useI18n()
const dialog = ref<HTMLElement | null>(null)

const isAnki = computed(() => props.action === 'anki_install')
const version = computed(() => props.ankiVersion || ANKI_PINNED_VERSION)
/** Checksums are only listed for the release they belong to. */
const installers = computed(() => (version.value === ANKI_PINNED_VERSION ? ANKI_INSTALLERS : []))
const titleId = computed(() => `consent-title-${props.action}`)
const pinnedSource = `${ANKICONNECT_REPOSITORY}/tree/${ANKICONNECT_COMMIT}/plugin`

onMounted(() => dialog.value?.focus())
</script>

<template>
  <div class="modal-overlay" @click.self="emit('cancel')">
    <div
      ref="dialog"
      class="modal-box consent-box"
      role="dialog"
      aria-modal="true"
      :aria-labelledby="titleId"
      tabindex="-1"
      @keydown.esc.stop.prevent="emit('cancel')"
    >
      <header class="modal-header">
        <h3 :id="titleId">
          {{ isAnki ? t('consent.titleAnki', { version }) : t('consent.titleAnkiconnect') }}
        </h3>
      </header>

      <div class="modal-body consent-body">
        <p class="consent-intro">{{ t('consent.intro') }}</p>

        <dl v-if="isAnki" class="consent-facts">
          <dt>{{ t('consent.whatHeading') }}</dt>
          <dd>{{ t('consent.ankiWhat', { version }) }}</dd>
          <dt>{{ t('consent.sourceHeading') }}</dt>
          <dd>
            {{ t('consent.ankiSource') }} <code>{{ ankiReleaseUrl(version) }}</code>
          </dd>
          <dt>{{ t('consent.verifyHeading') }}</dt>
          <dd>{{ t('consent.ankiVerify') }}</dd>
          <dt>{{ t('consent.writeHeading') }}</dt>
          <dd>
            <p>{{ t('consent.ankiWriteMac') }}</p>
            <p>{{ t('consent.ankiWriteWindows') }}</p>
          </dd>
        </dl>

        <dl v-else class="consent-facts">
          <dt>{{ t('consent.whatHeading') }}</dt>
          <dd>{{ t('consent.ankiconnectWhat') }}</dd>
          <dt>{{ t('consent.sourceHeading') }}</dt>
          <dd>
            {{ t('consent.ankiconnectSource') }}
            <code class="consent-pin">{{ pinnedSource }}</code>
          </dd>
          <dt>{{ t('consent.verifyHeading') }}</dt>
          <dd>{{ t('consent.ankiconnectVerify') }}</dd>
          <dt>{{ t('consent.writeHeading') }}</dt>
          <dd>
            <p>{{ t('consent.ankiconnectWrite') }}</p>
            <ul class="consent-paths">
              <li v-for="dir in ANKICONNECT_ADDON_DIRS" :key="dir.platform">
                {{ dir.platform }}: <code>{{ dir.path }}</code>
              </li>
            </ul>
          </dd>
        </dl>

        <details v-if="!isAnki || installers.length > 0" class="consent-checksums">
          <summary>{{ t('consent.checksums') }}</summary>
          <ul>
            <template v-if="isAnki">
              <li v-for="file in installers" :key="file.name">
                {{ file.platform }}: <code>{{ file.name }}</code>
                <code class="consent-hash">{{ file.sha256 }}</code>
              </li>
            </template>
            <template v-else>
              <li v-for="file in ANKICONNECT_FILES" :key="file.name">
                <code>plugin/{{ file.name }}</code>
                <code class="consent-hash">{{ file.sha256 }}</code>
              </li>
            </template>
          </ul>
        </details>
      </div>

      <footer class="modal-footer">
        <button
          type="button"
          class="btn btn-secondary"
          data-test="consent-cancel"
          @click="emit('cancel')"
        >
          {{ t('consent.cancel') }}
        </button>
        <button
          type="button"
          class="btn btn-primary"
          data-test="consent-agree"
          @click="emit('agree')"
        >
          {{ t('consent.agree') }}
        </button>
      </footer>
    </div>
  </div>
</template>
