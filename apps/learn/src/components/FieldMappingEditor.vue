<script setup lang="ts">
/**
 * Edits `LearnSettings.anki.field_mapping` (Milestone I, decision 6): which
 * field of the learner's Anki note type gets which part of a card. Export
 * writes these fields and import reads them back, so a deck whose note type
 * does not use the default names (Hanzi, Pinyin, Meaning, Sentence, Audio)
 * only transfers once the names here are its own.
 *
 * Every binding is a field name the learner types (Anki compares them
 * without regard to case) and the card value it holds; the audio field is
 * optional. What a save would refuse (a field without a name, no field for
 * the word or the meaning) is shown here, and the form does not save it.
 *
 * Focus never drops to the page: adding a field moves focus into its name,
 * removing one moves it to the next field's remove button, or to "Add a
 * field" once none is left.
 */
import { computed, nextTick, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import {
  CARD_FIELDS,
  CARD_FIELD_KEYS,
  defaultMapping,
  mappingProblems,
} from '../domain/fieldMapping'
import type { CardField, FieldMapping } from '../platform/types'

const mapping = defineModel<FieldMapping>({ required: true })

const { t } = useI18n()
const body = ref<HTMLTableSectionElement | null>(null)
const addButton = ref<HTMLButtonElement | null>(null)

const problems = computed(() => mappingProblems(mapping.value))

function update(edit: (fields: FieldMapping['fields']) => void): void {
  const fields = mapping.value.fields.map((binding) => ({ ...binding }))
  edit(fields)
  mapping.value = { ...mapping.value, fields }
}

function setName(index: number, event: Event): void {
  const name = (event.target as HTMLInputElement).value
  update((fields) => (fields[index].anki_field = name))
}

function setSource(index: number, event: Event): void {
  const source = (event.target as HTMLSelectElement).value as CardField
  update((fields) => (fields[index].source = source))
}

function setAudio(event: Event): void {
  const field = (event.target as HTMLInputElement).value
  mapping.value = { ...mapping.value, audio_field: field === '' ? null : field }
}

/** The first card value no field holds yet, so a new field starts out useful. */
function unusedSource(): CardField {
  const used = new Set(mapping.value.fields.map((binding) => binding.source))
  return CARD_FIELDS.find((source) => !used.has(source)) ?? 'notes'
}

/** The first element matching `selector` in the row of field `index`. */
function inRow<T extends HTMLElement>(index: number, selector: string): T | null {
  return body.value?.rows[index]?.querySelector<T>(selector) ?? null
}

function add(): void {
  const index = mapping.value.fields.length
  update((fields) => fields.push({ anki_field: '', source: unusedSource() }))
  void nextTick(() => inRow<HTMLInputElement>(index, 'input')?.focus())
}

function remove(index: number): void {
  update((fields) => fields.splice(index, 1))
  void nextTick(() => {
    const count = mapping.value.fields.length
    const next = count > 0 ? inRow<HTMLButtonElement>(Math.min(index, count - 1), 'button') : null
    ;(next ?? addButton.value)?.focus()
  })
}

function useDefault(): void {
  mapping.value = defaultMapping()
}
</script>

<template>
  <fieldset class="mapping-editor" data-test="mapping" aria-describedby="mapping-intro">
    <legend class="pane-subtitle">{{ t('mapping.title') }}</legend>
    <p id="mapping-intro" class="input-hint">{{ t('mapping.intro') }}</p>

    <table v-if="mapping.fields.length > 0" class="mapping-table">
      <thead>
        <tr>
          <th scope="col">{{ t('mapping.ankiField') }}</th>
          <th scope="col">{{ t('mapping.cardValue') }}</th>
          <td></td>
        </tr>
      </thead>
      <tbody ref="body">
        <tr v-for="(binding, index) in mapping.fields" :key="index" data-test="mapping-row">
          <td>
            <input
              type="text"
              spellcheck="false"
              autocomplete="off"
              :value="binding.anki_field"
              :aria-label="t('mapping.fieldName', { n: index + 1 })"
              :aria-invalid="binding.anki_field.trim() === ''"
              :class="{ 'has-error': binding.anki_field.trim() === '' }"
              :data-test="`mapping-name-${index}`"
              @input="setName(index, $event)"
            />
          </td>
          <td>
            <select
              class="mac-select"
              :value="binding.source"
              :aria-label="t('mapping.fieldValue', { n: index + 1 })"
              :data-test="`mapping-source-${index}`"
              @change="setSource(index, $event)"
            >
              <option v-for="source in CARD_FIELDS" :key="source" :value="source">
                {{ t(CARD_FIELD_KEYS[source]) }}
              </option>
            </select>
          </td>
          <td class="mapping-actions">
            <button
              type="button"
              class="btn btn-secondary btn-mini"
              :aria-label="t('mapping.removeField', { n: index + 1 })"
              :data-test="`mapping-remove-${index}`"
              @click="remove(index)"
            >
              {{ t('mapping.remove') }}
            </button>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-else class="input-hint" data-test="mapping-empty">{{ t('mapping.empty') }}</p>

    <div class="preset-row">
      <button
        ref="addButton"
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="mapping-add"
        @click="add"
      >
        {{ t('mapping.add') }}
      </button>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="mapping-default"
        @click="useDefault"
      >
        {{ t('mapping.useDefault') }}
      </button>
    </div>

    <ul v-if="problems.length > 0" class="field-error notice-list" data-test="mapping-problems">
      <li v-for="problem in problems" :key="problem">{{ t(problem) }}</li>
    </ul>

    <div class="field-group">
      <label for="mapping-audio">{{ t('mapping.audioField') }}</label>
      <input
        id="mapping-audio"
        type="text"
        spellcheck="false"
        autocomplete="off"
        :value="mapping.audio_field ?? ''"
        aria-describedby="mapping-audio-hint"
        data-test="mapping-audio"
        @input="setAudio"
      />
      <p id="mapping-audio-hint" class="input-hint">{{ t('mapping.audioHint') }}</p>
    </div>
  </fieldset>
</template>
