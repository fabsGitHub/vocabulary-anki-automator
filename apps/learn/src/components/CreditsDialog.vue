<script setup lang="ts">
/**
 * "Credits and licenses", opened from the menu: the data Learn ships under
 * someone else's license, each with its notice in full (domain/credits.ts).
 * CC-CEDICT's attribution is shown open; the license texts fold away.
 *
 * Focus moves to the close button and stays in the dialog until it closes
 * (Esc, the close button or a click beside it), then returns to where it was.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { DATA_CREDITS, paragraphs } from '../domain/credits'

const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const box = ref<HTMLElement | null>(null)
const closeButton = ref<HTMLButtonElement | null>(null)
let returnFocus: HTMLElement | null = null

function focusables(): HTMLElement[] {
  return [...(box.value?.querySelectorAll<HTMLElement>('button, summary') ?? [])]
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('close')
    return
  }
  if (event.key !== 'Tab') return
  const items = focusables()
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
  closeButton.value?.focus()
})

onBeforeUnmount(() => returnFocus?.focus())
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div
      ref="box"
      class="modal-box credits-box"
      role="dialog"
      aria-modal="true"
      aria-labelledby="credits-title"
      aria-describedby="credits-intro"
      data-test="credits"
      @keydown="onKeydown"
    >
      <header class="modal-header">
        <h2 id="credits-title">{{ t('credits.title') }}</h2>
        <button
          ref="closeButton"
          type="button"
          class="close-btn"
          :aria-label="t('credits.close')"
          :title="t('credits.close')"
          data-test="credits-close"
          @click="emit('close')"
        >
          &times;
        </button>
      </header>
      <div class="modal-body">
        <p id="credits-intro" class="input-hint">{{ t('credits.intro') }}</p>
        <section
          v-for="credit in DATA_CREDITS"
          :key="credit.file"
          class="credit"
          :aria-labelledby="`credit-${credit.file}`"
          :data-test="`credit-${credit.file}`"
        >
          <h3 :id="`credit-${credit.file}`">{{ credit.name }}</h3>
          <p>{{ t(credit.about) }}</p>
          <details :open="credit.file.startsWith('NOTICE-')">
            <summary>{{ t('credits.fullText') }}</summary>
            <div class="license-text">
              <p v-for="(paragraph, index) in paragraphs(credit.text)" :key="index">
                {{ paragraph }}
              </p>
            </div>
          </details>
        </section>
      </div>
    </div>
  </div>
</template>
