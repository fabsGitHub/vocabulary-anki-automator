<script setup lang="ts">
/**
 * The consent dialog shown before the install actions run (Milestone E,
 * docs/architecture.md § 8.4). It states, for each download, what is
 * downloaded, from where, how it is verified and what ends up where, and
 * only emits: recording the consent and starting the download stay with the
 * caller, so "Cancel" can never have started anything.
 *
 * The Anki tab asks for one action at a time; the setup assistant's "Set up
 * everything" asks once for every download it needs, and says in the `after`
 * slot what it does next.
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
  /** In the order they will run. */
  actions: readonly ConsentAction[]
  /** The Anki version `get_setup_status` reports. */
  ankiVersion: string
}>()

const emit = defineEmits<{ agree: []; cancel: [] }>()

const { t } = useI18n()
const dialog = ref<HTMLElement | null>(null)

const version = computed(() => props.ankiVersion || ANKI_PINNED_VERSION)
/** Checksums are only listed for the release they belong to. */
const installers = computed(() => (version.value === ANKI_PINNED_VERSION ? ANKI_INSTALLERS : []))
const several = computed(() => props.actions.length > 1)
const title = computed(() => {
  if (several.value) return t('consent.titleBoth', { version: version.value })
  return props.actions[0] === 'anki_install'
    ? t('consent.titleAnki', { version: version.value })
    : t('consent.titleAnkiconnect')
})
const titleId = computed(() => `consent-title-${props.actions.join('-')}`)
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
        <h3 :id="titleId">{{ title }}</h3>
      </header>

      <div class="modal-body consent-body">
        <p class="consent-intro">{{ t('consent.intro') }}</p>

        <section
          v-for="action in actions"
          :key="action"
          class="consent-download"
          :data-test="`consent-${action}`"
        >
          <template v-if="action === 'anki_install'">
            <h4 v-if="several">{{ t('consent.ankiHeading', { version }) }}</h4>
            <dl class="consent-facts">
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
            <details v-if="installers.length > 0" class="consent-checksums">
              <summary>{{ t('consent.checksums') }}</summary>
              <ul>
                <li v-for="file in installers" :key="file.name">
                  {{ file.platform }}: <code>{{ file.name }}</code>
                  <code class="consent-hash">{{ file.sha256 }}</code>
                </li>
              </ul>
            </details>
          </template>

          <template v-else>
            <h4 v-if="several">{{ t('consent.ankiconnectHeading') }}</h4>
            <dl class="consent-facts">
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
            <details class="consent-checksums">
              <summary>{{ t('consent.checksums') }}</summary>
              <ul>
                <li v-for="file in ANKICONNECT_FILES" :key="file.name">
                  <code>plugin/{{ file.name }}</code>
                  <code class="consent-hash">{{ file.sha256 }}</code>
                </li>
              </ul>
            </details>
          </template>
        </section>

        <slot name="after" />
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
