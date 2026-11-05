<script setup lang="ts">
/**
 * The options of a choice exercise (multiple choice, listening choice),
 * answered by click or by the option's number key. Once answered it marks
 * the correct option and, if different, the chosen one; the options' texts
 * are exactly the exercise's.
 */
import { useI18n } from 'vue-i18n'

import { useShortcuts } from '../../composables/useShortcuts'
import type { ChoiceOption } from '../../platform/types'

const props = defineProps<{
  options: ChoiceOption[]
  /** Language of the option texts. */
  lang: string
  chosen: number | null
  /** The correct option, once it may be shown. */
  correctIndex: number | null
  disabled: boolean
}>()

const emit = defineEmits<{ choose: [index: number] }>()

const { t } = useI18n()

function choose(index: number): void {
  if (!props.disabled) emit('choose', index)
}

useShortcuts((key) => {
  if (props.disabled || !/^[1-9]$/.test(key)) return false
  const index = Number(key) - 1
  if (index >= props.options.length) return false
  choose(index)
  return true
})

function state(index: number): 'correct' | 'wrong' | 'chosen' | null {
  if (props.correctIndex !== null && index === props.correctIndex) return 'correct'
  if (index === props.chosen) return props.correctIndex === null ? 'chosen' : 'wrong'
  return null
}
</script>

<template>
  <div class="choice-block">
    <ol class="choice-list" :aria-label="t('exercise.options')">
      <li v-for="(option, index) in options" :key="index">
        <button
          type="button"
          class="choice-option"
          :class="state(index) ? `is-${state(index)}` : ''"
          :disabled="disabled"
          :aria-pressed="chosen === index"
          :data-test="`option-${index}`"
          :data-state="state(index) ?? undefined"
          @click="choose(index)"
        >
          <kbd class="choice-key" aria-hidden="true">{{ index + 1 }}</kbd>
          <span class="choice-text" :lang="lang">{{ option.text }}</span>
          <span v-if="state(index) === 'correct'" class="choice-mark" aria-hidden="true">✓</span>
          <span v-else-if="state(index) === 'wrong'" class="choice-mark" aria-hidden="true">✗</span>
        </button>
      </li>
    </ol>
    <p class="input-hint">{{ t('exercise.choiceKeys') }}</p>
  </div>
</template>
