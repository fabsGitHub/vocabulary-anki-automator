<script setup lang="ts">
/**
 * The "Anki connection" tab: what is missing for Anki to receive cards, the
 * consent-gated ways to fix it (Milestone E), and where cards go: deck, note
 * type and field mapping (Milestones H and J5).
 *
 * Deck and note type stay editable while Anki is offline, as free text; the
 * lists Anki reports replace the text fields once it is online.
 */
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { ankiModelFieldNames, errorMessage } from '../api'
import ConsentDialog from '../components/ConsentDialog.vue'
import FieldMappingEditor from '../components/FieldMappingEditor.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'

const { t } = useI18n()
const store = useConfigStore()
const config = store.config
const issueText = useIssueText()
const anki = useAnkiConnectionContext()
const {
  state,
  status,
  activity,
  feedback,
  decks,
  models,
  canInstallAnki,
  ankiVersion,
  pendingConsent,
} = anki

const online = computed(() => state.value === 'online')
const busy = computed(() => activity.value !== null)
const modelOptions = computed(() => {
  const current = config.value.anki.model_name
  return current && !models.value.includes(current) ? [...models.value, current] : models.value
})

const modelFields = ref<string[] | null>(null)
const fieldsLoading = ref(false)
const fieldsError = ref<string | null>(null)
/** Note type -> its fields, for as long as Anki stays online. */
const fieldCache = new Map<string, string[]>()
let fieldsRequest = 0

async function loadModelFields(): Promise<void> {
  const request = ++fieldsRequest
  const model = config.value.anki.model_name.trim()
  fieldsError.value = null
  if (!online.value) fieldCache.clear()
  if (!online.value || !model) {
    modelFields.value = null
    fieldsLoading.value = false
    return
  }
  const cached = fieldCache.get(model)
  if (cached) {
    modelFields.value = cached
    fieldsLoading.value = false
    return
  }
  modelFields.value = null
  fieldsLoading.value = true
  try {
    const fields = await ankiModelFieldNames(model)
    if (request !== fieldsRequest) return
    fieldCache.set(model, fields)
    modelFields.value = fields
  } catch (err) {
    if (request === fieldsRequest) fieldsError.value = errorMessage(err)
  } finally {
    if (request === fieldsRequest) fieldsLoading.value = false
  }
}

watch([online, () => config.value.anki.model_name], () => void loadModelFields(), {
  immediate: true,
})
</script>

<template>
  <div class="pane-stack">
    <section class="pane-layout" data-test="anki-state" :data-state="state">
      <div v-if="state === 'checking'" class="onboarding-notice">
        <p class="spinner">{{ t('anki.checking') }}</p>
      </div>

      <div v-else-if="state === 'anki_missing'" class="onboarding-notice warning">
        <h4>{{ t('anki.missingTitle') }}</h4>
        <p>{{ t('anki.missingBody') }}</p>
        <div class="action-row-center">
          <button
            v-if="canInstallAnki"
            type="button"
            class="btn btn-primary"
            data-test="install-anki"
            :disabled="busy"
            @click="anki.requestInstall()"
          >
            {{
              activity === 'install'
                ? t('anki.installing')
                : t('anki.install', { version: ankiVersion })
            }}
          </button>
          <button
            type="button"
            class="btn btn-secondary"
            data-test="rescan"
            :disabled="busy"
            @click="anki.rescan()"
          >
            {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
          </button>
        </div>
        <p class="input-hint">
          {{ canInstallAnki ? t('anki.installManual') : t('anki.installUnsupported') }}
        </p>
      </div>

      <div v-else-if="state === 'plugin_missing'" class="onboarding-notice warning">
        <h4>{{ t('anki.pluginTitle') }}</h4>
        <p>{{ t('anki.pluginBody') }}</p>
        <div class="action-row-center">
          <button
            type="button"
            class="btn btn-primary"
            data-test="provision"
            :disabled="busy"
            @click="anki.requestProvision()"
          >
            {{ activity === 'provision' ? t('anki.provisioning') : t('anki.pluginInstall') }}
          </button>
          <button
            type="button"
            class="btn btn-secondary"
            data-test="rescan"
            :disabled="busy"
            @click="anki.rescan()"
          >
            {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
          </button>
        </div>
      </div>

      <div v-else-if="state === 'anki_closed'" class="onboarding-notice">
        <h4>{{ t('anki.closedTitle') }}</h4>
        <p>{{ t('anki.closedBody') }}</p>
        <div class="action-row-center">
          <button
            type="button"
            class="btn btn-primary"
            data-test="launch"
            :disabled="busy"
            @click="anki.launch()"
          >
            {{ activity === 'launch' ? t('anki.waiting') : t('anki.launch') }}
          </button>
          <button
            type="button"
            class="btn btn-secondary"
            data-test="rescan"
            :disabled="busy"
            @click="anki.rescan()"
          >
            {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
          </button>
        </div>
      </div>

      <div v-else class="success-notice-card online-row">
        <span>
          {{ t('anki.onlineTitle') }}
          <template v-if="status?.version"
            >· {{ t('anki.onlineVersion', { version: status.version }) }}</template
          >
        </span>
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="rescan"
          :disabled="busy"
          @click="anki.rescan()"
        >
          {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
        </button>
      </div>

      <p v-if="feedback" :class="['inline-result', feedback.tone]" data-test="anki-feedback">
        {{ t(feedback.key, feedback.params ?? {}) }}
      </p>
    </section>

    <section class="pane-layout">
      <h3 class="pane-title">{{ t('anki.targetTitle') }}</h3>
      <p v-if="!online && state !== 'checking'" class="input-hint">
        {{ t('anki.offlineEditing') }}
      </p>
      <div class="grid-row">
        <div class="field-group">
          <label for="anki-deck">{{ t('anki.deck') }}</label>
          <input
            id="anki-deck"
            v-model="config.anki.deck_name"
            type="text"
            class="mac-input"
            list="anki-decks"
            :class="{ 'has-error': issueText('anki.deck_name') }"
          />
          <datalist id="anki-decks">
            <option v-for="deck in decks" :key="deck" :value="deck" />
          </datalist>
          <span v-if="issueText('anki.deck_name')" class="field-error">{{
            issueText('anki.deck_name')
          }}</span>
          <span v-else class="input-hint">{{ t('anki.deckHint') }}</span>
        </div>
        <div class="field-group">
          <label for="anki-model">{{ t('anki.model') }}</label>
          <select
            v-if="online"
            id="anki-model"
            v-model="config.anki.model_name"
            class="mac-select"
            data-test="model-select"
            :class="{ 'has-error': issueText('anki.model_name') }"
          >
            <option v-for="model in modelOptions" :key="model" :value="model">
              {{ models.includes(model) ? model : t('anki.modelMissing', { model }) }}
            </option>
          </select>
          <input
            v-else
            id="anki-model"
            v-model="config.anki.model_name"
            type="text"
            class="mac-input"
            :class="{ 'has-error': issueText('anki.model_name') }"
          />
          <span v-if="issueText('anki.model_name')" class="field-error">{{
            issueText('anki.model_name')
          }}</span>
        </div>
      </div>
    </section>

    <section class="pane-layout">
      <FieldMappingEditor
        :model-fields="modelFields"
        :fields-loading="fieldsLoading"
        :fields-error="fieldsError"
      />
    </section>

    <ConsentDialog
      v-if="pendingConsent"
      :action="pendingConsent"
      :anki-version="ankiVersion"
      @agree="anki.confirmConsent()"
      @cancel="anki.cancelConsent()"
    />
  </div>
</template>
