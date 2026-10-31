<script setup lang="ts">
/**
 * The main window: settings in three tabs plus the history, the help dialog,
 * the UI-language switch and the update banner.
 *
 * It owns the two shared states, the settings store and the Anki connection,
 * and provides them to the tabs; the header badge and the Anki tab must show
 * the same connection. Startup only checks Anki (§ 8.4): nothing is installed
 * or launched without a click.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { onCaptureError, type UnlistenFn } from '../api'
import HelpModal from '../components/HelpModal.vue'
import HotkeyProfilesEditor from '../components/HotkeyProfilesEditor.vue'
import UpdateBanner from '../components/UpdateBanner.vue'
import { provideAnkiConnection, useAnkiConnection } from '../composables/useAnkiConnection'
import { provideConfigStore, useConfig } from '../composables/useConfig'
import { UI_LANGS, setLocale, toUiLang, type MessageKey, type UiLang } from '../i18n'
import AnkiConnectionView from './AnkiConnectionView.vue'
import CaptureSettings from './CaptureSettings.vue'
import HistoryView from './HistoryView.vue'

type TabId = 'capture' | 'profiles' | 'anki' | 'history'

interface Tab {
  id: TabId
  label: MessageKey
  /** Settings paths whose problems this tab shows. */
  paths: string[]
}

const TABS: Tab[] = [
  { id: 'capture', label: 'tabs.capture', paths: ['translation', 'hotkeys'] },
  { id: 'profiles', label: 'tabs.profiles', paths: ['hotkey_profiles'] },
  { id: 'anki', label: 'tabs.anki', paths: ['anki'] },
  { id: 'history', label: 'tabs.history', paths: [] },
]

const UI_LANG_LABELS: Record<UiLang, string> = { en: 'English', de: 'Deutsch' }

/** How long a success or capture-error message stays. */
const MESSAGE_MS = 5000

const { t } = useI18n()
const store = useConfig()
provideConfigStore(store)
const anki = useAnkiConnection({ onConsentChanged: (consent) => store.setConsent(consent) })
provideAnkiConnection(anki)

const { config, loaded, loading, loadError, saving, saveError, uiLangError, dirty, isValid } = store
const { state: ankiState, decks } = anki

const activeTab = ref<TabId>('capture')
const showHelp = ref(false)
const saveNotice = ref<'saved' | 'invalid' | null>(null)
const captureError = ref<string | null>(null)
const uiLang = computed(() => toUiLang(config.value.ui_lang))

const badge = computed(() => {
  if (ankiState.value === 'checking') return { tone: 'checking', text: t('status.checking') }
  return ankiState.value === 'online'
    ? { tone: 'online', text: t('status.online') }
    : { tone: 'offline', text: t('status.offline') }
})

const timers = new Set<ReturnType<typeof setTimeout>>()
function later(ms: number, run: () => void): void {
  const timer = setTimeout(() => {
    timers.delete(timer)
    run()
  }, ms)
  timers.add(timer)
}

function tabHasIssues(tab: Tab): boolean {
  return loaded.value && tab.paths.some((path) => store.hasIssuesUnder(path))
}

watch(isValid, (valid) => {
  if (valid && saveNotice.value === 'invalid') saveNotice.value = null
})

watch(
  () => config.value.ui_lang,
  (lang) => {
    setLocale(lang)
    document.title = t('app.title')
  },
  { immediate: true },
)

async function switchUiLang(event: Event): Promise<void> {
  const lang = (event.target as HTMLSelectElement).value
  setLocale(lang)
  await store.persistUiLang(lang)
}

async function save(): Promise<void> {
  saveNotice.value = null
  if (!isValid.value) {
    saveNotice.value = 'invalid'
    return
  }
  if (await store.save()) {
    saveNotice.value = 'saved'
    later(MESSAGE_MS, () => {
      if (saveNotice.value === 'saved') saveNotice.value = null
    })
  }
}

function revert(): void {
  saveNotice.value = null
  store.revert()
}

function onWindowFocus(): void {
  if (anki.activity.value === null && ankiState.value !== 'checking') void anki.check()
}

let stopCaptureErrors: UnlistenFn | null = null
let unmounted = false

onMounted(async () => {
  window.addEventListener('focus', onWindowFocus)
  void store.load()
  void anki.check()
  try {
    const stop = await onCaptureError(({ message }) => {
      captureError.value = message
      later(MESSAGE_MS * 2, () => {
        if (captureError.value === message) captureError.value = null
      })
    })
    if (unmounted) stop()
    else stopCaptureErrors = stop
  } catch (err) {
    console.warn('[main] could not listen for capture errors:', err)
  }
})

onBeforeUnmount(() => {
  unmounted = true
  window.removeEventListener('focus', onWindowFocus)
  stopCaptureErrors?.()
  for (const timer of timers) clearTimeout(timer)
})
</script>

<template>
  <main class="view app-shell app-fade-in">
    <header class="mac-header">
      <div class="header-titles">
        <h2>{{ t('app.title') }}</h2>
        <p>{{ t('app.subtitle') }}</p>
      </div>
      <div class="controls-cluster">
        <span :class="['status-badge', badge.tone]" data-test="anki-badge">● {{ badge.text }}</span>
        <select
          class="mac-select compact-select"
          data-test="ui-lang"
          :value="uiLang"
          :aria-label="t('app.uiLanguage')"
          :title="t('app.uiLanguage')"
          @change="switchUiLang"
        >
          <option v-for="lang in UI_LANGS" :key="lang" :value="lang">
            {{ UI_LANG_LABELS[lang] }}
          </option>
        </select>
        <button
          type="button"
          class="help-circle"
          data-test="help"
          :aria-label="t('app.help')"
          :title="t('app.help')"
          @click="showHelp = true"
        >
          ?
        </button>
      </div>
    </header>

    <UpdateBanner />

    <div v-if="captureError" class="notice error toast-row" role="alert" data-test="capture-error">
      <span>{{ t('app.captureFailed', { message: captureError }) }}</span>
      <button
        type="button"
        class="close-btn"
        :aria-label="t('app.dismiss')"
        @click="captureError = null"
      >
        &times;
      </button>
    </div>
    <p v-if="uiLangError" class="notice error" role="alert">
      {{ t('settings.uiLangFailed', { error: uiLangError }) }}
    </p>

    <nav class="segmented-bar" role="tablist">
      <button
        v-for="tab in TABS"
        :key="tab.id"
        type="button"
        role="tab"
        :aria-selected="activeTab === tab.id"
        :class="['segment-btn', { active: activeTab === tab.id }]"
        :data-test="`tab-${tab.id}`"
        @click="activeTab = tab.id"
      >
        {{ t(tab.label) }}
        <span v-if="tabHasIssues(tab)" class="tab-issue-dot" :title="t('tabs.hasIssues')" />
      </button>
    </nav>

    <div class="pane-scroll">
      <HistoryView v-if="activeTab === 'history'" />
      <template v-else-if="loaded">
        <CaptureSettings v-if="activeTab === 'capture'" />
        <section v-else-if="activeTab === 'profiles'" class="pane-layout">
          <HotkeyProfilesEditor :decks="decks" />
        </section>
        <AnkiConnectionView v-else />
      </template>
      <div v-else-if="loadError" class="onboarding-notice warning" data-test="load-error">
        <p>{{ t('app.loadFailed', { error: loadError }) }}</p>
        <button type="button" class="btn btn-primary" :disabled="loading" @click="store.load()">
          {{ t('app.retry') }}
        </button>
      </div>
      <p v-else class="input-hint spinner">{{ t('app.loading') }}</p>
    </div>

    <footer v-if="activeTab !== 'history' && loaded" class="footer-action-bar">
      <div v-if="saveError" class="interactive-toast error" role="alert">
        {{ t('settings.saveFailed', { error: saveError }) }}
      </div>
      <div v-else-if="saveNotice === 'invalid'" class="interactive-toast error" role="alert">
        {{ t('settings.fixIssues') }}
      </div>
      <div v-else-if="saveNotice === 'saved'" class="interactive-toast success" role="status">
        {{ t('settings.saved') }}
      </div>
      <span v-else-if="dirty" class="dirty-note">{{ t('settings.unsaved') }}</span>
      <button
        v-if="dirty"
        type="button"
        class="btn btn-secondary"
        data-test="revert"
        :disabled="saving"
        @click="revert"
      >
        {{ t('settings.revert') }}
      </button>
      <button
        type="button"
        class="btn btn-primary"
        data-test="save"
        :disabled="saving || !dirty"
        @click="save"
      >
        {{ saving ? t('settings.saving') : t('settings.save') }}
      </button>
    </footer>

    <HelpModal v-if="showHelp" @close="showHelp = false" />
  </main>
</template>
