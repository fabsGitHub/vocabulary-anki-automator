<script setup lang="ts">
/** The setup and usage instructions behind the "?" button of the main window. */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

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
      </div>
    </div>
  </div>
</template>
