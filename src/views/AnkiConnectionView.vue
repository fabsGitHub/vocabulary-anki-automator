<script setup lang="ts">
/**
 * The "Anki connection" tab: what is missing for Anki to receive cards, the
 * consent-gated ways to fix it (Milestone E), and where cards go: deck, note
 * type, field mapping and reverse card (Milestones H and J5). With languages
 * listed under "My languages", these apply only to text in other languages,
 * which the tab says.
 *
 * The field mapping is matched by field name until the user customizes it
 * (`null`, the default), with the same preview, "Customize" and "Stop
 * customizing" as each "My languages" entry (`MappingChoiceEditor`); the
 * preview is for the main language pair, and each card is matched for its
 * own. Offline, "Customize" starts from the Apple-style preset.
 *
 * Deck and note type stay editable while Anki is offline, as free text; the
 * lists Anki reports replace the text fields once it is online. While it is
 * online, "Create the recommended note type" (`RecommendedNoteType`) adds the
 * note type with a field for every value and selects it here, matched by
 * name (`selectRecommendedNoteType`).
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import ConsentDialog from '../components/ConsentDialog.vue'
import MappingChoiceEditor from '../components/MappingChoiceEditor.vue'
import RecommendedNoteType from '../components/RecommendedNoteType.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { useAutoMatches, type AutoMatchRequest } from '../composables/useAutoMatches'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import { useModelFields } from '../composables/useModelFields'
import {
  appleStyleMapping,
  needsChineseNoteType,
  recommendedNoteTypeAction,
  selectRecommendedNoteType,
} from '../domain/fieldMapping'

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
const matched = computed(() => config.value.anki.field_mapping === null)

/** A customized mapping is edited against the note type's fields. */
const modelFields = useModelFields(() => (matched.value ? [] : [config.value.anki.model_name]))
const fields = computed(() => modelFields.state(config.value.anki.model_name))

const matchRequest = (): AutoMatchRequest => ({
  model: config.value.anki.model_name,
  sourceLang: config.value.translation.source_lang,
  targetLang: config.value.translation.target_lang,
})
const autoMatches = useAutoMatches(() => (matched.value ? [matchRequest()] : []))
const suggestion = computed(() => autoMatches.state(matchRequest()))

const chineseWarning = computed(() => needsChineseNoteType(config.value, suggestion.value.match))
const fallback = appleStyleMapping()
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
      <p
        v-if="config.learning.languages.length > 0"
        class="input-hint"
        data-test="anki-fallback-hint"
      >
        {{ t('anki.otherLanguagesHint') }}
      </p>
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
      <RecommendedNoteType
        :model-name="config.anki.model_name"
        @use="(noteType) => selectRecommendedNoteType(config.anki, noteType)"
      />
    </section>

    <section class="pane-layout">
      <div v-if="matched" class="pane-title-row">
        <h3 class="pane-title">{{ t('mapping.title') }}</h3>
        <span class="badge badge-muted" data-test="mapping-badge">{{ t('mapping.auto') }}</span>
      </div>
      <MappingChoiceEditor
        v-model:mapping="config.anki.field_mapping"
        :model-name="config.anki.model_name"
        :source-lang="config.translation.source_lang"
        :target-lang="config.translation.target_lang"
        :suggestion="suggestion"
        :fields="fields"
        :fallback="fallback"
        :issue="issueText('anki.field_mapping')"
      >
        <template #notices>
          <div
            v-if="chineseWarning"
            class="notice warning"
            role="alert"
            data-test="chinese-warning"
          >
            {{ t('mapping.chineseWarning', { button: t(recommendedNoteTypeAction(models)) }) }}
          </div>
        </template>
        <template #options>
          <label class="checkbox-row">
            <input
              v-model="config.anki.create_reverse"
              type="checkbox"
              data-test="create-reverse"
            />
            <span>{{ t('mapping.createReverse') }}</span>
          </label>
        </template>
      </MappingChoiceEditor>
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
