<script setup lang="ts">
/**
 * Edits `learning.languages`, the "My languages" tab: the languages the
 * default hotkey recognizes, each filed into its own deck and note type with
 * its own field mapping and reverse card (docs/architecture.md § 8.1). The
 * list order is the order of the approval window's language chips.
 *
 * A new entry is prefilled by `newLearnedLanguage` and starts with the
 * mapping `null`: the note type's fields are matched by name whenever a card
 * is pushed, so a changed note type is matched again. While Anki is online
 * the entry previews that match (`suggest_field_mapping`); the preview lives
 * here, never in the form, so it dirties nothing. "Customize" saves a copy of
 * the match (offline: of the global mapping) for the usual mapping editor,
 * and "Match by field name again" goes back to `null`.
 */
import { computed, reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import {
  errorMessage,
  suggestFieldMapping,
  type AutoMatch,
  type CardField,
  type FieldMapping,
  type LearnedLanguage,
} from '../api'
import FieldMappingEditor from './FieldMappingEditor.vue'
import LanguageSelect from './LanguageSelect.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import { useModelFields } from '../composables/useModelFields'
import { bindsSource } from '../domain/fieldMapping'
import { isChinese, isSupportedLang, normalizeLang } from '../domain/language'
import { DEFAULT_DECKS, availableLanguages, newLearnedLanguage } from '../domain/learning'
import { LANGUAGE_NAME_KEYS } from '../i18n'

const { t } = useI18n()
const store = useConfigStore()
const config = store.config
const issueText = useIssueText()
const { state, decks, models } = useAnkiConnectionContext()

const online = computed(() => state.value === 'online')
const languages = computed(() => config.value.learning.languages)
const canAdd = computed(() => newLearnedLanguage(config.value) !== null)

/** A saved mapping is edited against its note type's fields, read once per note type. */
const modelFields = useModelFields(() =>
  languages.value.filter((entry) => entry.field_mapping !== null).map((entry) => entry.model_name),
)

interface Suggestion {
  match: AutoMatch | null
  loading: boolean
  error: string | null
}

/** What matching by name gives, per note type and pair; forgotten when Anki goes offline. */
const suggestions = reactive(new Map<string, Suggestion>())
/** Bumped when Anki goes offline, so an answer from before is dropped. */
let suggestionRun = 0

function suggestionKey(entry: LearnedLanguage): string {
  return [entry.model_name.trim(), entry.lang, config.value.translation.target_lang].join('\n')
}

function suggest(entry: LearnedLanguage): void {
  const model = entry.model_name.trim()
  const key = suggestionKey(entry)
  const current = suggestions.get(key)
  if (!model || (current && (current.loading || current.match))) return
  const run = suggestionRun
  suggestions.set(key, { match: null, loading: true, error: null })
  suggestFieldMapping(model, entry.lang, config.value.translation.target_lang).then(
    (match) => {
      if (run === suggestionRun) suggestions.set(key, { match, loading: false, error: null })
    },
    (err: unknown) => {
      if (run === suggestionRun) {
        suggestions.set(key, { match: null, loading: false, error: errorMessage(err) })
      }
    },
  )
}

const autoEntries = () => languages.value.filter((entry) => entry.field_mapping === null)

/** Each entry with what the template shows next to it. */
const cards = computed(() =>
  languages.value.map((entry, index) => {
    const suggestion = suggestions.get(suggestionKey(entry))
    return {
      entry,
      index,
      matching: suggestion?.loading ?? false,
      matchError: suggestion?.error ?? null,
      match: suggestion?.match ?? null,
      fields: modelFields.state(entry.model_name),
    }
  }),
)

watch(
  [online, () => autoEntries().map(suggestionKey).join('\n\n')],
  ([now]) => {
    if (!now) {
      suggestionRun++
      suggestions.clear()
      return
    }
    for (const entry of autoEntries()) suggest(entry)
  },
  { immediate: true },
)

function languageName(code: string): string {
  const lang = normalizeLang(code)
  return isSupportedLang(lang) ? t(LANGUAGE_NAME_KEYS[lang]) : code
}

function deckPlaceholder(entry: LearnedLanguage): string {
  const lang = normalizeLang(entry.lang)
  const deck = isSupportedLang(lang) ? DEFAULT_DECKS[lang] : config.value.anki.deck_name
  return t('learning.deckPlaceholder', { deck })
}

function modelOptions(entry: LearnedLanguage): string[] {
  const current = entry.model_name
  return current && !models.value.includes(current) ? [...models.value, current] : models.value
}

function sourceLabel(source: CardField): string {
  return t(`mapping.sources.${source}`)
}

/** A saved Chinese mapping that leaves the pinyin out (Milestone J5, per entry). */
function needsPinyin(entry: LearnedLanguage): boolean {
  return (
    isChinese(entry.lang) &&
    entry.field_mapping !== null &&
    !bindsSource(entry.field_mapping, 'pinyin')
  )
}

function copyMapping(mapping: FieldMapping): FieldMapping {
  return {
    fields: mapping.fields.map((binding) => ({ ...binding })),
    audio_field: mapping.audio_field,
  }
}

function addLanguage(): void {
  const entry = newLearnedLanguage(config.value)
  if (entry) config.value.learning.languages.push(entry)
}

function removeLanguage(index: number): void {
  config.value.learning.languages.splice(index, 1)
}

function customize(entry: LearnedLanguage): void {
  const match = online.value ? suggestions.get(suggestionKey(entry))?.match : null
  entry.field_mapping = copyMapping(match?.mapping ?? config.value.anki.field_mapping)
}

function rematch(entry: LearnedLanguage): void {
  entry.field_mapping = null
}

function issue(index: number, field: keyof LearnedLanguage): string | null {
  return issueText(`learning.languages.${index}.${field}`)
}
</script>

<template>
  <section class="learned-editor">
    <div class="pane-title-row">
      <h3 class="pane-title">{{ t('learning.title') }}</h3>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="add-language"
        :disabled="!canAdd"
        :title="canAdd ? undefined : t('learning.allAdded')"
        @click="addLanguage"
      >
        {{ t('learning.add') }}
      </button>
    </div>
    <p class="input-hint">{{ t('learning.intro') }}</p>
    <p class="input-hint" data-test="native-language">
      {{ t('learning.native', { lang: languageName(config.translation.target_lang) }) }}
    </p>

    <p v-if="languages.length === 0" class="empty-state" data-test="no-languages">
      {{ t('learning.empty') }}
    </p>

    <datalist id="learned-decks">
      <option v-for="deck in decks" :key="deck" :value="deck" />
    </datalist>

    <article
      v-for="{ entry, index, matching, matchError, match, fields } in cards"
      :key="index"
      class="profile-card"
      data-test="learned-language"
    >
      <div class="grid-row">
        <div class="field-group">
          <label :for="`learned-${index}-lang`">{{ t('learning.language') }}</label>
          <LanguageSelect
            :id="`learned-${index}-lang`"
            v-model="entry.lang"
            :options="availableLanguages(config, index)"
            :invalid="!!issue(index, 'lang')"
          />
          <span v-if="issue(index, 'lang')" class="field-error">{{ issue(index, 'lang') }}</span>
        </div>
        <div class="field-group">
          <label :for="`learned-${index}-deck`">{{ t('learning.deck') }}</label>
          <input
            :id="`learned-${index}-deck`"
            v-model="entry.deck_name"
            type="text"
            class="mac-input"
            list="learned-decks"
            data-test="learned-deck"
            :class="{ 'has-error': issue(index, 'deck_name') }"
            :placeholder="deckPlaceholder(entry)"
          />
          <span v-if="issue(index, 'deck_name')" class="field-error">
            {{ issue(index, 'deck_name') }}
          </span>
        </div>
      </div>

      <div class="grid-row">
        <div class="field-group">
          <label :for="`learned-${index}-model`">{{ t('learning.model') }}</label>
          <select
            v-if="online"
            :id="`learned-${index}-model`"
            v-model="entry.model_name"
            class="mac-select"
            data-test="learned-model"
            :class="{ 'has-error': issue(index, 'model_name') }"
          >
            <option v-for="model in modelOptions(entry)" :key="model" :value="model">
              {{ models.includes(model) ? model : t('anki.modelMissing', { model }) }}
            </option>
          </select>
          <input
            v-else
            :id="`learned-${index}-model`"
            v-model="entry.model_name"
            type="text"
            class="mac-input"
            data-test="learned-model"
            :class="{ 'has-error': issue(index, 'model_name') }"
          />
          <span v-if="issue(index, 'model_name')" class="field-error">
            {{ issue(index, 'model_name') }}
          </span>
        </div>
        <label class="checkbox-row">
          <input v-model="entry.create_reverse" type="checkbox" data-test="learned-reverse" />
          <span>{{ t('mapping.createReverse') }}</span>
        </label>
      </div>

      <div
        v-if="needsPinyin(entry)"
        class="notice warning"
        role="alert"
        data-test="chinese-warning"
      >
        {{ t('mapping.chineseWarning') }}
      </div>

      <details class="learned-mapping">
        <summary>
          {{ t('learning.mapping') }}
          <span
            :class="['badge', { 'badge-muted': entry.field_mapping === null }]"
            data-test="mapping-badge"
          >
            {{
              entry.field_mapping === null ? t('learning.mappingAuto') : t('learning.mappingCustom')
            }}
          </span>
        </summary>

        <div v-if="entry.field_mapping === null" class="learned-mapping-body">
          <p class="input-hint">
            {{ t('learning.mappingAutoHint', { model: entry.model_name }) }}
          </p>
          <template v-if="online">
            <p v-if="matching" class="input-hint">{{ t('mapping.matching') }}</p>
            <div v-else-if="matchError" class="notice error" data-test="preview-error">
              {{ t('mapping.matchFailed', { error: matchError }) }}
            </div>
            <template v-else-if="match">
              <table class="mapping-table" data-test="mapping-preview">
                <thead>
                  <tr>
                    <th scope="col">{{ t('mapping.ankiField') }}</th>
                    <th scope="col">{{ t('mapping.cardValue') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="binding in match.mapping.fields"
                    :key="binding.anki_field"
                    :data-field="binding.anki_field"
                  >
                    <td>{{ binding.anki_field }}</td>
                    <td>{{ sourceLabel(binding.source) }}</td>
                  </tr>
                </tbody>
              </table>
              <p class="input-hint" data-test="preview-audio">
                {{
                  match.mapping.audio_field
                    ? t('learning.audioTo', { field: match.mapping.audio_field })
                    : t('learning.noAudio')
                }}
              </p>
              <p v-if="match.guessed.length > 0" data-test="matched-guessed">
                {{ t('mapping.matchedGuessed') }}
                <code v-for="field in match.guessed" :key="field" class="field-chip">{{
                  field
                }}</code>
              </p>
              <p v-if="match.unmatched.length > 0" data-test="matched-unmatched">
                {{ t('mapping.matchedUnmatched') }}
                <code v-for="field in match.unmatched" :key="field" class="field-chip">{{
                  field
                }}</code>
              </p>
            </template>
          </template>
          <p v-else class="input-hint" data-test="mapping-offline">
            {{ t('learning.mappingOffline', { model: entry.model_name }) }}
          </p>
          <div>
            <button
              type="button"
              class="btn btn-secondary btn-mini"
              data-test="customize-mapping"
              :disabled="online && matching"
              @click="customize(entry)"
            >
              {{ t('learning.customize') }}
            </button>
          </div>
        </div>

        <div v-else class="learned-mapping-body">
          <FieldMappingEditor
            v-model:mapping="entry.field_mapping"
            :model-name="entry.model_name"
            :model-fields="fields.fields"
            :fields-loading="fields.loading"
            :fields-error="fields.error"
            :source-lang="entry.lang"
            :target-lang="config.translation.target_lang"
            :id-prefix="`learned-${index}-mapping`"
          />
          <div>
            <button
              type="button"
              class="btn btn-secondary btn-mini"
              data-test="rematch-mapping"
              @click="rematch(entry)"
            >
              {{ t('learning.rematch') }}
            </button>
          </div>
        </div>
      </details>
      <span v-if="issue(index, 'field_mapping')" class="field-error" data-test="mapping-issue">
        {{ issue(index, 'field_mapping') }}
      </span>

      <div class="learned-footer">
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="remove-language"
          @click="removeLanguage(index)"
        >
          {{ t('learning.remove') }}
        </button>
      </div>
    </article>
  </section>
</template>
