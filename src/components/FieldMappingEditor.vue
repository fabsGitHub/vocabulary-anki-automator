<script setup lang="ts">
/**
 * Edits `anki.field_mapping` and `anki.create_reverse` (Milestones H and J5).
 *
 * With Anki online it lists every field of the selected note type and binds
 * each to a card value or to nothing; offline it can still edit the bindings
 * already saved. Bindings for fields the note type lacks are kept and
 * flagged, because AnkiConnect drops their values without an error.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { CardField } from '../api'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import {
  CARD_FIELDS,
  MAPPING_PRESETS,
  bindingFor,
  effectiveBindings,
  missingFields,
  needsChineseNoteType,
  withAudioField,
  withBinding,
  type MappingPresetId,
} from '../domain/fieldMapping'

const props = defineProps<{
  /** The note type's fields, or `null` while they are unknown (Anki offline). */
  modelFields: string[] | null
  fieldsLoading?: boolean
  fieldsError?: string | null
}>()

const { t } = useI18n()
const store = useConfigStore()
const config = store.config
const issueText = useIssueText()

const mapping = computed(() => config.value.anki.field_mapping)
const modelName = computed(() => config.value.anki.model_name)

interface Row {
  field: string
  source: CardField | ''
  inModel: boolean
}

const rows = computed<Row[]>(() => {
  const fields = props.modelFields
  const bound = effectiveBindings(mapping.value)
  if (!fields) return bound.map((b) => ({ field: b.anki_field, source: b.source, inModel: true }))
  const known = (name: string) => fields.some((field) => field.toLowerCase() === name.toLowerCase())
  return [
    ...fields.map((field) => ({
      field,
      source: bindingFor(mapping.value, field)?.source ?? ('' as const),
      inModel: true,
    })),
    ...bound
      .filter((b) => !known(b.anki_field))
      .map((b) => ({ field: b.anki_field, source: b.source, inModel: false })),
  ]
})

const missing = computed(() =>
  props.modelFields ? missingFields(mapping.value, props.modelFields) : [],
)
const chineseWarning = computed(() => needsChineseNoteType(config.value))
const audioOptions = computed(() => {
  const fields = props.modelFields ?? []
  const current = mapping.value.audio_field
  return current && !fields.includes(current) ? [...fields, current] : fields
})

function setSource(field: string, value: string): void {
  const source = value === '' ? null : (value as CardField)
  config.value.anki.field_mapping = withBinding(mapping.value, field, source)
}

function setAudio(value: string): void {
  config.value.anki.field_mapping = withAudioField(mapping.value, value)
}

function applyPreset(id: MappingPresetId): void {
  config.value.anki.field_mapping = MAPPING_PRESETS[id]()
}

function sourceLabel(source: CardField): string {
  return t(`mapping.sources.${source}`)
}
</script>

<template>
  <section class="mapping-editor">
    <div class="pane-title-row">
      <h3 class="pane-title">{{ t('mapping.title') }}</h3>
      <div class="preset-row">
        <span class="input-hint">{{ t('mapping.presets') }}</span>
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="preset-apple"
          @click="applyPreset('apple')"
        >
          {{ t('mapping.presetApple') }}
        </button>
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="preset-chinese"
          @click="applyPreset('chinese')"
        >
          {{ t('mapping.presetChinese') }}
        </button>
      </div>
    </div>

    <p class="input-hint">{{ t('mapping.intro', { model: modelName }) }}</p>

    <div v-if="chineseWarning" class="notice warning" role="alert" data-test="chinese-warning">
      {{ t('mapping.chineseWarning') }}
    </div>

    <p v-if="fieldsLoading" class="input-hint">
      {{ t('mapping.loadingFields', { model: modelName }) }}
    </p>
    <div v-else-if="fieldsError" class="notice error" data-test="fields-error">
      {{ t('mapping.fieldsFailed', { model: modelName, error: fieldsError }) }}
    </div>
    <p v-if="!modelFields && !fieldsLoading" class="input-hint" data-test="mapping-offline">
      {{ t('mapping.offline') }}
    </p>

    <table class="mapping-table">
      <thead>
        <tr>
          <th scope="col">{{ t('mapping.ankiField') }}</th>
          <th scope="col">{{ t('mapping.cardValue') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.field" :data-field="row.field">
          <td>
            <span class="mapping-field">{{ row.field }}</span>
            <span v-if="!row.inModel" class="badge badge-warning">{{
              t('mapping.notInModel')
            }}</span>
          </td>
          <td>
            <select
              class="mac-select"
              :value="row.source"
              :aria-label="`${row.field}: ${t('mapping.cardValue')}`"
              @change="setSource(row.field, ($event.target as HTMLSelectElement).value)"
            >
              <option value="">{{ t('mapping.none') }}</option>
              <option v-for="source in CARD_FIELDS" :key="source" :value="source">
                {{ sourceLabel(source) }}
              </option>
            </select>
          </td>
        </tr>
        <tr v-if="rows.length === 0">
          <td colspan="2" class="input-hint">{{ t('mapping.noBindings') }}</td>
        </tr>
      </tbody>
    </table>

    <div v-if="missing.length > 0" class="notice warning" data-test="missing-fields">
      {{ t('mapping.missing', { model: modelName }) }}
      <code v-for="field in missing" :key="field" class="field-chip">{{ field }}</code>
    </div>
    <p v-if="issueText('anki.field_mapping')" class="field-error">
      {{ issueText('anki.field_mapping') }}
    </p>

    <div class="grid-row">
      <div class="field-group">
        <label for="mapping-audio">{{ t('mapping.audioField') }}</label>
        <select
          v-if="modelFields"
          id="mapping-audio"
          class="mac-select"
          data-test="audio-field"
          :value="mapping.audio_field ?? ''"
          @change="setAudio(($event.target as HTMLSelectElement).value)"
        >
          <option value="">{{ t('mapping.noAudio') }}</option>
          <option v-for="field in audioOptions" :key="field" :value="field">{{ field }}</option>
        </select>
        <input
          v-else
          id="mapping-audio"
          type="text"
          class="mac-input"
          data-test="audio-field"
          :value="mapping.audio_field ?? ''"
          @change="setAudio(($event.target as HTMLInputElement).value)"
        />
      </div>
      <label class="checkbox-row">
        <input v-model="config.anki.create_reverse" type="checkbox" data-test="create-reverse" />
        <span>{{ t('mapping.createReverse') }}</span>
      </label>
    </div>
  </section>
</template>
