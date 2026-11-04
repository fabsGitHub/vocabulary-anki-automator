<script setup lang="ts">
/**
 * Edits one field mapping, bound with `v-model:mapping` (Milestones H and
 * J5): the Anki tab's `anki.field_mapping`.
 *
 * With Anki online it lists every field of the note type and binds each to a
 * card value or to nothing; offline it can still edit the bindings already
 * saved. Bindings for fields the note type lacks are kept and flagged,
 * because AnkiConnect drops their values without an error.
 *
 * "Match by field name" asks the backend (`suggest_field_mapping`) to bind
 * every field of the note type by its name for the `sourceLang` ->
 * `targetLang` pair, and replaces the mapping with the result; the fields it
 * could only fill by position, or not at all, are listed for as long as the
 * mapping is the matched one.
 *
 * The parent owns what lives around the mapping: the `notices` slot shows
 * under the intro, the `options` slot next to the audio field.
 */
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import {
  errorMessage,
  suggestFieldMapping,
  type AutoMatch,
  type CardField,
  type FieldMapping,
} from '../api'
import {
  CARD_FIELDS,
  MAPPING_PRESETS,
  bindingFor,
  effectiveBindings,
  missingFields,
  withAudioField,
  withBinding,
  type MappingPresetId,
} from '../domain/fieldMapping'

const props = withDefaults(
  defineProps<{
    /** The note type the mapping is for. */
    modelName: string
    /** The note type's fields, or `null` while they are unknown (Anki offline). */
    modelFields: string[] | null
    fieldsLoading?: boolean
    fieldsError?: string | null
    /** The language pair "Match by field name" matches for. */
    sourceLang: string
    targetLang: string
    /** The validation message for the mapping, if any. */
    issue?: string | null
    /** Prefixes the element ids, so several editors can share a page. */
    idPrefix?: string
  }>(),
  { fieldsLoading: false, fieldsError: null, issue: null, idPrefix: 'mapping' },
)

const mapping = defineModel<FieldMapping>('mapping', { required: true })

const { t } = useI18n()

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
const audioOptions = computed(() => {
  const fields = props.modelFields ?? []
  const current = mapping.value.audio_field
  return current && !fields.includes(current) ? [...fields, current] : fields
})
const audioId = computed(() => `${props.idPrefix}-audio`)

const matching = ref(false)
const matched = ref<AutoMatch | null>(null)
const matchError = ref<string | null>(null)
/** Bumped by every match and every change of note type or pair, so a late answer is dropped. */
let matchRequest = 0

function sameMapping(a: FieldMapping, b: FieldMapping): boolean {
  return (
    a.audio_field === b.audio_field &&
    a.fields.length === b.fields.length &&
    a.fields.every(
      (binding, index) =>
        binding.anki_field === b.fields[index].anki_field &&
        binding.source === b.fields[index].source,
    )
  )
}

/** The last match's leftovers, while the mapping is still what it produced. */
const matchResult = computed(() =>
  matched.value && sameMapping(matched.value.mapping, mapping.value) ? matched.value : null,
)

watch(
  () => [props.modelName, props.sourceLang, props.targetLang],
  () => {
    matchRequest++
    matching.value = false
    matched.value = null
    matchError.value = null
  },
)

async function matchByName(): Promise<void> {
  const request = ++matchRequest
  matching.value = true
  matchError.value = null
  try {
    const result = await suggestFieldMapping(props.modelName, props.sourceLang, props.targetLang)
    if (request !== matchRequest) return
    matched.value = result
    mapping.value = result.mapping
  } catch (err) {
    if (request === matchRequest) matchError.value = errorMessage(err)
  } finally {
    if (request === matchRequest) matching.value = false
  }
}

function setSource(field: string, value: string): void {
  const source = value === '' ? null : (value as CardField)
  mapping.value = withBinding(mapping.value, field, source)
}

function setAudio(value: string): void {
  mapping.value = withAudioField(mapping.value, value)
}

function applyPreset(id: MappingPresetId): void {
  mapping.value = MAPPING_PRESETS[id]()
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
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="match-by-name"
          :disabled="!modelFields || matching"
          :title="modelFields ? undefined : t('mapping.matchOffline')"
          @click="matchByName"
        >
          {{ matching ? t('mapping.matching') : t('mapping.matchByName') }}
        </button>
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

    <slot name="notices" />

    <p v-if="fieldsLoading" class="input-hint">
      {{ t('mapping.loadingFields', { model: modelName }) }}
    </p>
    <div v-else-if="fieldsError" class="notice error" data-test="fields-error">
      {{ t('mapping.fieldsFailed', { model: modelName, error: fieldsError }) }}
    </div>
    <p v-if="!modelFields && !fieldsLoading" class="input-hint" data-test="mapping-offline">
      {{ t('mapping.offline') }}
    </p>

    <div v-if="matchError" class="notice error" role="alert" data-test="match-error">
      {{ t('mapping.matchFailed', { error: matchError }) }}
    </div>
    <div
      v-if="matchResult && (matchResult.guessed.length > 0 || matchResult.unmatched.length > 0)"
      class="notice"
      role="status"
      data-test="match-result"
    >
      <p v-if="matchResult.guessed.length > 0" data-test="matched-guessed">
        {{ t('mapping.matchedGuessed') }}
        <code v-for="field in matchResult.guessed" :key="field" class="field-chip">{{
          field
        }}</code>
      </p>
      <p v-if="matchResult.unmatched.length > 0" data-test="matched-unmatched">
        {{ t('mapping.matchedUnmatched') }}
        <code v-for="field in matchResult.unmatched" :key="field" class="field-chip">{{
          field
        }}</code>
      </p>
    </div>

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
    <p v-if="issue" class="field-error">{{ issue }}</p>

    <div class="grid-row">
      <div class="field-group">
        <label :for="audioId">{{ t('mapping.audioField') }}</label>
        <select
          v-if="modelFields"
          :id="audioId"
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
          :id="audioId"
          type="text"
          class="mac-input"
          data-test="audio-field"
          :value="mapping.audio_field ?? ''"
          @change="setAudio(($event.target as HTMLInputElement).value)"
        />
      </div>
      <slot name="options" />
    </div>
  </section>
</template>
