<script setup lang="ts">
/**
 * Settings and Anki portability (Milestone I, decision 6: the app never
 * locks anyone out of their Anki collection).
 *
 * The form edits the parts of `LearnSettings` a learner chooses: the meaning
 * language of new HSK words, the Anki deck and note type, reverse cards.
 * Everything else in the settings (the AnkiConnect URL, the field mapping) is
 * saved back exactly as it was loaded. Export takes its target from the
 * *saved* settings (§ 10), so it waits until the form is saved; import reads
 * the deck typed next to its button. Both need AnkiConnect, which only the
 * desktop app has (`capabilities.anki`).
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import SyncReportView from '../components/SyncReportView.vue'
import { usePlatform } from '../context'
import { errorMessage } from '../domain/errors'
import type { AnkiSyncReport, LearnSettings } from '../platform/types'

/** The meaning languages, in the order the select lists them. */
const MEANING_LANGS: readonly LearnSettings['meaning_lang'][] = ['de', 'en']

const { t } = useI18n()
const platform = usePlatform()
const ankiAvailable = platform.capabilities.anki

const saved = ref<LearnSettings | null>(null)
const draft = ref<LearnSettings | null>(null)
const loading = ref(false)
const loadError = ref<string | null>(null)
const saving = ref(false)
const savedNotice = ref(false)
const saveError = ref<string | null>(null)

const exporting = ref(false)
const exportReport = ref<AnkiSyncReport | null>(null)
const exportError = ref<string | null>(null)
const importDeck = ref('')
const importing = ref(false)
const importReport = ref<AnkiSyncReport | null>(null)
const importError = ref<string | null>(null)

function copy(settings: LearnSettings): LearnSettings {
  return JSON.parse(JSON.stringify(settings)) as LearnSettings
}

/** What a save sends: the draft with the names trimmed. */
const toSave = computed<LearnSettings | null>(() => {
  if (!draft.value) return null
  const next = copy(draft.value)
  next.anki.deck_name = next.anki.deck_name.trim()
  next.anki.model_name = next.anki.model_name.trim()
  return next
})
const dirty = computed(
  () => saved.value !== null && JSON.stringify(toSave.value) !== JSON.stringify(saved.value),
)
const deckMissing = computed(() => toSave.value?.anki.deck_name === '')
const modelMissing = computed(() => toSave.value?.anki.model_name === '')
const canSave = computed(() => dirty.value && !deckMissing.value && !modelMissing.value)
const busy = computed(() => saving.value || exporting.value || importing.value)

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    const settings = await platform.getSettings()
    saved.value = copy(settings)
    draft.value = copy(settings)
    importDeck.value = settings.anki.deck_name
  } catch (err) {
    loadError.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

async function save(): Promise<void> {
  const next = toSave.value
  if (!next || !canSave.value || busy.value) return
  saving.value = true
  saveError.value = null
  savedNotice.value = false
  try {
    await platform.saveSettings(next)
    saved.value = copy(next)
    draft.value = copy(next)
    savedNotice.value = true
  } catch (err) {
    saveError.value = errorMessage(err)
  } finally {
    saving.value = false
  }
}

function revert(): void {
  if (saved.value) draft.value = copy(saved.value)
  saveError.value = null
}

async function runExport(): Promise<void> {
  if (busy.value || dirty.value || !ankiAvailable) return
  exporting.value = true
  exportError.value = null
  exportReport.value = null
  try {
    exportReport.value = await platform.ankiExport()
  } catch (err) {
    exportError.value = errorMessage(err)
  } finally {
    exporting.value = false
  }
}

async function runImport(): Promise<void> {
  const deck = importDeck.value.trim()
  if (busy.value || !deck || !ankiAvailable) return
  importing.value = true
  importError.value = null
  importReport.value = null
  try {
    importReport.value = await platform.ankiImport(deck)
  } catch (err) {
    importError.value = errorMessage(err)
  } finally {
    importing.value = false
  }
}

function meaningLabel(lang: LearnSettings['meaning_lang']): string {
  return lang === 'de' ? t('languages.de') : t('languages.en')
}

onMounted(() => void load())
</script>

<template>
  <div class="pane-stack" data-test="anki">
    <p v-if="loading && !draft" class="input-hint spinner">{{ t('settings.loading') }}</p>
    <div
      v-else-if="loadError && !draft"
      class="onboarding-notice warning"
      data-test="settings-error"
    >
      <p role="alert">{{ t('settings.loadFailed', { error: loadError }) }}</p>
      <button type="button" class="btn btn-primary" data-test="settings-retry" @click="load">
        {{ t('app.retry') }}
      </button>
    </div>

    <template v-else-if="draft">
      <form class="pane-layout" novalidate data-test="settings-form" @submit.prevent="save">
        <h2 class="pane-title">{{ t('settings.title') }}</h2>
        <div class="field-group">
          <label for="settings-meaning">{{ t('settings.meaningLang') }}</label>
          <select
            id="settings-meaning"
            v-model="draft.meaning_lang"
            class="mac-select"
            data-test="meaning-lang"
            aria-describedby="settings-meaning-hint"
          >
            <option v-for="lang in MEANING_LANGS" :key="lang" :value="lang">
              {{ meaningLabel(lang) }}
            </option>
          </select>
          <p id="settings-meaning-hint" class="input-hint">{{ t('settings.meaningHint') }}</p>
        </div>

        <h3 class="pane-subtitle">{{ t('settings.ankiTitle') }}</h3>
        <div class="grid-row">
          <div class="field-group">
            <label for="settings-deck">{{ t('settings.deck') }}</label>
            <input
              id="settings-deck"
              v-model="draft.anki.deck_name"
              type="text"
              data-test="deck"
              :class="{ 'has-error': deckMissing }"
              :aria-invalid="deckMissing"
              :aria-describedby="deckMissing ? 'settings-deck-error' : undefined"
            />
            <p v-if="deckMissing" id="settings-deck-error" class="field-error">
              {{ t('settings.required') }}
            </p>
          </div>
          <div class="field-group">
            <label for="settings-model">{{ t('settings.model') }}</label>
            <input
              id="settings-model"
              v-model="draft.anki.model_name"
              type="text"
              data-test="model"
              :class="{ 'has-error': modelMissing }"
              :aria-invalid="modelMissing"
              :aria-describedby="modelMissing ? 'settings-model-error' : undefined"
            />
            <p v-if="modelMissing" id="settings-model-error" class="field-error">
              {{ t('settings.required') }}
            </p>
          </div>
        </div>
        <label class="checkbox-row">
          <input v-model="draft.anki.create_reverse" type="checkbox" data-test="reverse" />
          {{ t('settings.reverse') }}
        </label>

        <div class="footer-action-bar">
          <div v-if="saveError" class="interactive-toast error" role="alert" data-test="save-error">
            {{ t('settings.saveFailed', { error: saveError }) }}
          </div>
          <div
            v-else-if="savedNotice && !dirty"
            class="interactive-toast success"
            role="status"
            data-test="saved"
          >
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
            type="submit"
            class="btn btn-primary"
            data-test="save"
            :disabled="!canSave || busy"
          >
            {{ saving ? t('settings.saving') : t('settings.save') }}
          </button>
        </div>
      </form>

      <section class="pane-layout" data-test="anki-sync">
        <h2 class="pane-title">{{ t('anki.title') }}</h2>
        <p v-if="!ankiAvailable" class="notice" data-test="anki-unavailable">
          {{ t('anki.unavailable') }}
        </p>
        <template v-else>
          <p class="input-hint">{{ t('anki.intro') }}</p>

          <div class="sync-row">
            <button
              type="button"
              class="btn btn-primary"
              data-test="export"
              :disabled="busy || dirty"
              :aria-describedby="dirty ? 'anki-save-first' : undefined"
              @click="runExport"
            >
              {{ exporting ? t('anki.exporting') : t('anki.export') }}
            </button>
            <p v-if="dirty" id="anki-save-first" class="input-hint" data-test="save-first">
              {{ t('anki.saveFirst') }}
            </p>
          </div>
          <div aria-live="polite">
            <SyncReportView v-if="exportReport" :report="exportReport" mode="export" />
          </div>
          <div v-if="exportError" class="notice error" role="alert" data-test="export-error">
            <p>{{ t('anki.exportFailed', { error: exportError }) }}</p>
            <p class="notice-detail">{{ t('anki.reachHint') }}</p>
          </div>

          <form class="sync-row" novalidate @submit.prevent="runImport">
            <div class="field-group sync-deck">
              <label for="anki-import-deck">{{ t('anki.importDeck') }}</label>
              <input
                id="anki-import-deck"
                v-model="importDeck"
                type="text"
                data-test="import-deck"
              />
            </div>
            <button
              type="submit"
              class="btn btn-secondary"
              data-test="import"
              :disabled="busy || !importDeck.trim()"
            >
              {{ importing ? t('anki.importing') : t('anki.import') }}
            </button>
          </form>
          <div aria-live="polite">
            <SyncReportView v-if="importReport" :report="importReport" mode="import" />
          </div>
          <div v-if="importError" class="notice error" role="alert" data-test="import-error">
            <p>{{ t('anki.importFailed', { error: importError }) }}</p>
            <p class="notice-detail">{{ t('anki.reachHint') }}</p>
          </div>
        </template>
      </section>
    </template>
  </div>
</template>
