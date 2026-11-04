<script setup lang="ts">
/**
 * Edits `learning.languages`, the "My languages" tab: the languages the
 * default hotkey recognizes, each filed into its own deck and note type with
 * its own field mapping and reverse card (docs/architecture.md § 8.1). The
 * list order is the order of the approval window's language chips.
 *
 * A new entry is prefilled by `newLearnedLanguage`, and a change of its
 * language brings the new language's deck and reverse default along while the
 * old ones are untouched (`switchLanguage`). It starts with the mapping
 * `null`: the note type's fields are matched by name whenever a card is
 * pushed, so a changed note type is matched again. `MappingChoiceEditor`,
 * shared with the Anki tab's global mapping, previews that match while Anki
 * is online and offers "Customize" (offline: a copy of the global mapping,
 * or the Apple-style preset while that one is matched by name too) and
 * "Stop customizing". A Chinese entry is checked for a Pinyin field in the
 * mapping it will use: the saved one, or else the preview (offline, an entry
 * matched by name stays silent). Each entry offers to create the recommended
 * note type (`RecommendedNoteType`) and select it for that entry.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { LearnedLanguage } from '../api'
import LanguageSelect from './LanguageSelect.vue'
import MappingChoiceEditor from './MappingChoiceEditor.vue'
import RecommendedNoteType from './RecommendedNoteType.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { useAutoMatches, type AutoMatchRequest } from '../composables/useAutoMatches'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import { useModelFields } from '../composables/useModelFields'
import {
  appleStyleMapping,
  lacksPinyin,
  recommendedNoteTypeAction,
  selectRecommendedNoteType,
} from '../domain/fieldMapping'
import { isChinese, isSupportedLang, normalizeLang } from '../domain/language'
import {
  DEFAULT_DECKS,
  availableLanguages,
  newLearnedLanguage,
  switchLanguage,
} from '../domain/learning'
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

/** An entry's mapping is matched for its own language and the user's. */
function matchRequest(entry: LearnedLanguage): AutoMatchRequest {
  return {
    model: entry.model_name,
    sourceLang: entry.lang,
    targetLang: config.value.translation.target_lang,
  }
}

/** The previews of every entry matched by name, once per note type and pair. */
const autoMatches = useAutoMatches(() =>
  languages.value.filter((entry) => entry.field_mapping === null).map(matchRequest),
)

/** What "Customize" copies offline: the global mapping, or the preset it stands for. */
const fallback = computed(() => config.value.anki.field_mapping ?? appleStyleMapping())

/** Each entry with what the template shows next to it. */
const cards = computed(() =>
  languages.value.map((entry, index) => {
    const suggestion = autoMatches.state(matchRequest(entry))
    return {
      entry,
      index,
      suggestion,
      needsPinyin: isChinese(entry.lang) && lacksPinyin(entry.field_mapping, suggestion.match),
      fields: modelFields.state(entry.model_name),
    }
  }),
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

function addLanguage(): void {
  const entry = newLearnedLanguage(config.value)
  if (entry) config.value.learning.languages.push(entry)
}

function removeLanguage(index: number): void {
  config.value.learning.languages.splice(index, 1)
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
      v-for="{ entry, index, suggestion, needsPinyin, fields } in cards"
      :key="index"
      class="profile-card"
      data-test="learned-language"
    >
      <div class="grid-row">
        <div class="field-group">
          <label :for="`learned-${index}-lang`">{{ t('learning.language') }}</label>
          <LanguageSelect
            :id="`learned-${index}-lang`"
            :model-value="entry.lang"
            :options="availableLanguages(config, index)"
            :invalid="!!issue(index, 'lang')"
            @update:model-value="(lang: string) => switchLanguage(config, entry, lang)"
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

      <RecommendedNoteType
        :model-name="entry.model_name"
        @use="(noteType) => selectRecommendedNoteType(entry, noteType)"
      />

      <div v-if="needsPinyin" class="notice warning" role="alert" data-test="chinese-warning">
        {{ t('mapping.chineseWarning', { button: t(recommendedNoteTypeAction(models)) }) }}
      </div>

      <details class="learned-mapping">
        <summary>
          {{ t('learning.mapping') }}
          <span
            :class="['badge', { 'badge-muted': entry.field_mapping === null }]"
            data-test="mapping-badge"
          >
            {{ entry.field_mapping === null ? t('mapping.auto') : t('mapping.custom') }}
          </span>
        </summary>

        <MappingChoiceEditor
          v-model:mapping="entry.field_mapping"
          class="learned-mapping-body"
          :model-name="entry.model_name"
          :source-lang="entry.lang"
          :target-lang="config.translation.target_lang"
          :suggestion="suggestion"
          :fields="fields"
          :fallback="fallback"
          :id-prefix="`learned-${index}-mapping`"
        />
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
