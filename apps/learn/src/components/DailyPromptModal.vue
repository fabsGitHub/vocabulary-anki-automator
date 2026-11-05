<script setup lang="ts">
/**
 * "How many new words do you want to learn today?" (Milestone L2).
 *
 * The intro says where new words come from: captures and Anki imports first,
 * then the HSK word list (docs/architecture.md § 7.1). The web build has no
 * captures or imports (`web`), so it names only the word list.
 *
 * It only collects the answer: "Start" emits the count (0–50), "Not today"
 * (also Esc) emits a dismissal, which the core records as reviews only for
 * today (docs/architecture.md § 7.1); the dialog says so, so closing it is
 * never a silent choice. Focus stays inside while it is open and returns to
 * where it was when it closes. While an answer is saved both buttons are
 * `aria-disabled` rather than disabled, so a failed save leaves focus (and
 * with it Esc) in the dialog instead of on the inert page behind it.
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { MAX_NEW_COUNT, MIN_NEW_COUNT, QUICK_PICKS } from '../composables/useDailyPrompt'

const props = defineProps<{
  initialCount: number
  busy: boolean
  error: string | null
  web?: boolean
}>()

const emit = defineEmits<{ start: [count: number]; dismiss: [] }>()

const { t } = useI18n()
const box = ref<HTMLElement | null>(null)
const input = ref<HTMLInputElement | null>(null)
const count = ref<number | string>(props.initialCount)
let returnFocus: HTMLElement | null = null

const parsed = computed(() => (typeof count.value === 'number' ? count.value : Number.NaN))
const valid = computed(
  () =>
    Number.isInteger(parsed.value) &&
    parsed.value >= MIN_NEW_COUNT &&
    parsed.value <= MAX_NEW_COUNT,
)

function pick(value: number): void {
  count.value = value
}

function start(): void {
  if (valid.value && !props.busy) emit('start', parsed.value)
}

function dismiss(): void {
  if (!props.busy) emit('dismiss')
}

function focusables(): HTMLElement[] {
  return [
    ...(box.value?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ) ?? []),
  ]
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    dismiss()
    return
  }
  if (event.key !== 'Tab') return
  const items = focusables()
  if (items.length === 0) return
  const first = items[0]
  const last = items[items.length - 1]
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}

onMounted(() => {
  returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
  input.value?.focus()
  input.value?.select()
})

onBeforeUnmount(() => returnFocus?.focus())
</script>

<template>
  <div class="modal-overlay">
    <div
      ref="box"
      class="modal-box prompt-box"
      role="dialog"
      aria-modal="true"
      aria-labelledby="daily-prompt-title"
      aria-describedby="daily-prompt-intro"
      data-test="daily-prompt"
      @keydown="onKeydown"
    >
      <form class="prompt-form" novalidate @submit.prevent="start">
        <header class="modal-header">
          <h2 id="daily-prompt-title">{{ t('prompt.title') }}</h2>
        </header>

        <div class="modal-body prompt-body">
          <p id="daily-prompt-intro" class="input-hint">
            {{ t(web ? 'prompt.introWeb' : 'prompt.intro') }}
          </p>

          <div class="field-group">
            <label for="daily-prompt-count">{{ t('prompt.countLabel') }}</label>
            <input
              id="daily-prompt-count"
              ref="input"
              v-model.number="count"
              type="number"
              inputmode="numeric"
              :min="MIN_NEW_COUNT"
              :max="MAX_NEW_COUNT"
              step="1"
              data-test="prompt-count"
              :class="{ 'has-error': !valid }"
              :aria-invalid="!valid"
              :aria-describedby="valid ? 'daily-prompt-count-hint' : 'daily-prompt-count-error'"
            />
            <p v-if="valid" id="daily-prompt-count-hint" class="input-hint">
              {{ t('prompt.countHint') }}
            </p>
            <p v-else id="daily-prompt-count-error" class="field-error" data-test="prompt-invalid">
              {{ t('prompt.invalid') }}
            </p>
          </div>

          <div class="quick-picks" role="group" :aria-label="t('prompt.quickPicks')">
            <button
              v-for="value in QUICK_PICKS"
              :key="value"
              type="button"
              class="btn btn-secondary btn-mini"
              :class="{ 'is-selected': count === value }"
              :aria-pressed="count === value"
              :data-test="`quick-pick-${value}`"
              @click="pick(value)"
            >
              {{ value }}
            </button>
          </div>

          <p class="notice" data-test="not-today-hint">{{ t('prompt.notTodayHint') }}</p>
          <p v-if="error" class="notice error" role="alert" data-test="prompt-error">
            {{ t('prompt.failed', { error }) }}
          </p>
        </div>

        <footer class="modal-footer">
          <button
            type="button"
            class="btn btn-secondary"
            data-test="prompt-dismiss"
            :aria-disabled="busy || undefined"
            @click="dismiss"
          >
            {{ t('prompt.notToday') }}
          </button>
          <button
            type="submit"
            class="btn btn-primary"
            data-test="prompt-start"
            :disabled="!valid"
            :aria-disabled="busy || undefined"
          >
            {{ busy ? t('prompt.saving') : t('prompt.start') }}
          </button>
        </footer>
      </form>
    </div>
  </div>
</template>
