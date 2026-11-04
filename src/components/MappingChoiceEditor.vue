<script setup lang="ts">
/**
 * One field mapping that is either matched by field name or customized,
 * bound with `v-model:mapping` (docs/architecture.md § 8.1): each "My
 * languages" entry's own mapping.
 *
 * `null` means the note type's fields are matched by name whenever a card is
 * pushed, so a changed note type is matched again and nothing is saved.
 * While Anki is online this shows that match, the `suggestion` the parent
 * keeps (`useAutoMatches`), so it dirties nothing. "Customize" saves a copy
 * of the match (offline: of `fallback`) and hands it to the usual
 * `FieldMappingEditor`; "Stop customizing" goes back to `null`.
 *
 * The parent's `notices` slot shows under the intro and its `options` slot
 * at the end, in both states.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { CardField, FieldMapping } from '../api'
import FieldMappingEditor from './FieldMappingEditor.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import type { AutoMatchState } from '../composables/useAutoMatches'
import type { ModelFieldsState } from '../composables/useModelFields'
import { copyMapping } from '../domain/fieldMapping'

const props = withDefaults(
  defineProps<{
    /** The note type the mapping is for. */
    modelName: string
    /** The pair a customized mapping's "Match by field name" matches for. */
    sourceLang: string
    targetLang: string
    /** The match by field name of `modelName` (`useAutoMatches`). */
    suggestion: AutoMatchState
    /** The note type's fields, which a customized mapping is edited against (`useModelFields`). */
    fields: ModelFieldsState
    /** What "Customize" copies while no match is known (Anki offline). */
    fallback: FieldMapping
    /** The validation message for a customized mapping, if any. */
    issue?: string | null
    /** Prefixes the editor's element ids, so several can share a page. */
    idPrefix?: string
  }>(),
  { issue: null, idPrefix: 'mapping' },
)

const mapping = defineModel<FieldMapping | null>('mapping', { required: true })

const { t } = useI18n()
const { state } = useAnkiConnectionContext()
const online = computed(() => state.value === 'online')
const match = computed(() => props.suggestion.match)

function sourceLabel(source: CardField): string {
  return t(`mapping.sources.${source}`)
}

function customize(): void {
  const known = online.value ? match.value : null
  mapping.value = copyMapping(known?.mapping ?? props.fallback)
}
</script>

<template>
  <div v-if="mapping === null" class="mapping-choice" data-test="mapping-auto">
    <p class="input-hint">{{ t('mapping.autoHint', { model: modelName }) }}</p>
    <slot name="notices" />
    <template v-if="online">
      <p v-if="suggestion.loading" class="input-hint">{{ t('mapping.matching') }}</p>
      <div v-else-if="suggestion.error" class="notice error" data-test="preview-error">
        {{ t('mapping.matchFailed', { error: suggestion.error }) }}
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
              ? t('mapping.audioTo', { field: match.mapping.audio_field })
              : t('mapping.noAudioField')
          }}
        </p>
        <p v-if="match.guessed.length > 0" data-test="matched-guessed">
          {{ t('mapping.matchedGuessed') }}
          <code v-for="field in match.guessed" :key="field" class="field-chip">{{ field }}</code>
        </p>
        <p v-if="match.unmatched.length > 0" data-test="matched-unmatched">
          {{ t('mapping.matchedUnmatched') }}
          <code v-for="field in match.unmatched" :key="field" class="field-chip">{{ field }}</code>
        </p>
      </template>
    </template>
    <p v-else class="input-hint" data-test="mapping-offline">
      {{ t('mapping.autoOffline', { model: modelName }) }}
    </p>
    <div>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="customize-mapping"
        :disabled="online && suggestion.loading"
        @click="customize"
      >
        {{ t('mapping.customize') }}
      </button>
    </div>
    <slot name="options" />
  </div>

  <div v-else class="mapping-choice" data-test="mapping-custom">
    <FieldMappingEditor
      v-model:mapping="mapping"
      :model-name="modelName"
      :model-fields="fields.fields"
      :fields-loading="fields.loading"
      :fields-error="fields.error"
      :source-lang="sourceLang"
      :target-lang="targetLang"
      :issue="issue"
      :id-prefix="idPrefix"
    >
      <template #notices><slot name="notices" /></template>
      <template #options><slot name="options" /></template>
    </FieldMappingEditor>
    <div>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="rematch-mapping"
        @click="mapping = null"
      >
        {{ t('mapping.rematch') }}
      </button>
    </div>
  </div>
</template>
