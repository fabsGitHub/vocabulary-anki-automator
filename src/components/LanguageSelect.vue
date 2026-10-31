<script setup lang="ts">
/**
 * A capture-language select over the canonical codes (docs/architecture.md
 * § 3). A stored code outside that list is still shown, so the validation
 * message next to it has something to point at.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { SUPPORTED_LANGS, isSupportedLang } from '../domain/language'
import { LANGUAGE_NAME_KEYS } from '../i18n'

defineProps<{ id?: string; invalid?: boolean; disabled?: boolean }>()

const model = defineModel<string>({ required: true })
const { t } = useI18n()
const unknown = computed(() => !isSupportedLang(model.value))
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
    <option v-for="code in SUPPORTED_LANGS" :key="code" :value="code">
      {{ t(LANGUAGE_NAME_KEYS[code]) }}
    </option>
    <option v-if="unknown" :value="model">{{ model }}</option>
  </select>
</template>
