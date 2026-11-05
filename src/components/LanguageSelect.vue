<script setup lang="ts">
/**
 * A capture-language select over the canonical codes (docs/architecture.md
 * § 3), or over the subset in `options`. A stored code outside the list is
 * still shown, so the validation message next to it has something to point
 * at.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { SUPPORTED_LANGS, isSupportedLang, type LanguageCode } from '../domain/language'
import { LANGUAGE_NAME_KEYS } from '../i18n'

const props = defineProps<{
  id?: string
  invalid?: boolean
  disabled?: boolean
  /** The languages offered, in this order; every supported one when absent. */
  options?: readonly LanguageCode[]
}>()

const model = defineModel<string>({ required: true })
const { t } = useI18n()
const offered = computed(() => props.options ?? SUPPORTED_LANGS)
const unknown = computed(() => !offered.value.some((code) => code === model.value))
</script>

<template>
  <select
    :id="id"
    v-model="model"
    class="mac-select"
    :class="{ 'has-error': invalid }"
    :aria-invalid="invalid ? 'true' : undefined"
    :disabled="disabled"
  >
    <option v-for="code in offered" :key="code" :value="code">
      {{ t(LANGUAGE_NAME_KEYS[code]) }}
    </option>
    <option v-if="unknown" :value="model">
      {{ isSupportedLang(model) ? t(LANGUAGE_NAME_KEYS[model]) : model }}
    </option>
  </select>
</template>
