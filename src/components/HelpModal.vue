<script setup lang="ts">
/**
 * The setup and usage instructions behind the "?" button of the main window,
 * then the credits and licenses of the bundled HSK data (domain/credits.ts):
 * CC-CEDICT's attribution shown open, the license texts folded away.
 */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { DATA_CREDITS, paragraphs } from '../domain/credits'

const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const box = ref<HTMLElement | null>(null)

onMounted(() => box.value?.focus())
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div
      ref="box"
      class="modal-box"
      role="dialog"
      aria-modal="true"
      aria-labelledby="help-title"
      tabindex="-1"
      @keydown.esc.stop.prevent="emit('close')"
    >
      <header class="modal-header">
        <h3 id="help-title">{{ t('help.title') }}</h3>
        <button
          type="button"
          class="close-btn"
          :aria-label="t('help.close')"
          :title="t('help.close')"
          @click="emit('close')"
        >
          &times;
        </button>
      </header>
      <div class="modal-body">
        <ol class="help-list">
          <li>
            <strong>{{ t('help.step1Title') }}</strong>
            <p>{{ t('help.step1Body') }}</p>
          </li>
          <li>
            <strong>{{ t('help.step2Title') }}</strong>
            <p>{{ t('help.step2Body') }}</p>
          </li>
          <li>
            <strong>{{ t('help.step3Title') }}</strong>
            <p>{{ t('help.step3Body') }}</p>
          </li>
          <li>
            <strong>{{ t('help.step4Title') }}</strong>
            <p>{{ t('help.step4Body') }}</p>
          </li>
        </ol>
        <section class="credits" aria-labelledby="credits-title" data-test="credits">
          <h4 id="credits-title">{{ t('credits.title') }}</h4>
          <p>{{ t('credits.intro') }}</p>
          <div
            v-for="credit in DATA_CREDITS"
            :key="credit.file"
            class="credit"
            :data-test="`credit-${credit.file}`"
          >
            <strong>{{ credit.name }}</strong>
            <p>{{ t(credit.about) }}</p>
            <details :open="credit.file.startsWith('NOTICE-')">
              <summary>{{ t('credits.fullText') }}</summary>
              <div class="license-text">
                <p v-for="(paragraph, index) in paragraphs(credit.text)" :key="index">
                  {{ paragraph }}
                </p>
              </div>
            </details>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
