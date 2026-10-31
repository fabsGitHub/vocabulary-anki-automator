<script setup lang="ts">
/**
 * The "Capture & Language" tab: the default hotkey's language pair and
 * shortcut, the capture length limit and the translation service (Milestone
 * C: provider-neutral, with the optional MyMemory contact e-mail), plus a
 * test capture that runs the real pipeline without the OS hotkey.
 */
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { captureText, errorMessage } from '../api'
import LanguageSelect from '../components/LanguageSelect.vue'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import { MAX_CHARS_LIMIT, TRANSLATION_PROVIDERS } from '../domain/validation'
import type { MessageKey } from '../i18n'

const { t } = useI18n()
const store = useConfigStore()
const config = store.config
const issueText = useIssueText()

const PROVIDER_LABELS: Record<(typeof TRANSLATION_PROVIDERS)[number], MessageKey> = {
  mymemory: 'capture.providerMymemory',
}

const contactEmail = computed({
  get: () => config.value.translation.contact_email ?? '',
  set: (value: string) => {
    config.value.translation.contact_email = value === '' ? null : value
  },
})

const tryText = ref('')
const tryProfile = ref('')
const trying = ref(false)
const tryResult = ref<{ tone: 'success' | 'error'; text: string } | null>(null)

/** `capture_text` resolves the profile in the saved settings, so unsaved profiles are not offered. */
const tryProfiles = computed(() => store.saved.value.hotkey_profiles)

watch(tryProfiles, (profiles) => {
  if (tryProfile.value && !profiles.some((profile) => profile.id === tryProfile.value)) {
    tryProfile.value = ''
  }
})

async function tryCapture(): Promise<void> {
  const text = tryText.value.trim()
  if (!text || trying.value) return
  trying.value = true
  tryResult.value = null
  try {
    await captureText(text, tryProfile.value || null)
    tryResult.value = { tone: 'success', text: t('capture.trySent') }
  } catch (err) {
    tryResult.value = { tone: 'error', text: t('capture.tryFailed', { error: errorMessage(err) }) }
  } finally {
    trying.value = false
  }
}

/**
 * Enter captures, except the Enter that commits an input method's
 * composition (typing hanzi with a pinyin keyboard): that one belongs to the
 * IME, and WebKit reports it with keyCode 229 rather than `isComposing`.
 */
function onTryEnter(event: KeyboardEvent): void {
  if (event.isComposing || event.keyCode === 229) return
  event.preventDefault()
  void tryCapture()
}
</script>

<template>
  <div class="pane-stack">
    <section class="pane-layout">
      <h3 class="pane-title">{{ t('capture.languagesTitle') }}</h3>
      <div class="grid-row">
        <div class="field-group">
          <label for="source-lang">{{ t('capture.sourceLang') }}</label>
          <LanguageSelect
            id="source-lang"
            v-model="config.translation.source_lang"
            :invalid="!!issueText('translation.source_lang')"
          />
          <span v-if="issueText('translation.source_lang')" class="field-error">
            {{ issueText('translation.source_lang') }}
          </span>
        </div>
        <div class="field-group">
          <label for="target-lang">{{ t('capture.targetLang') }}</label>
          <LanguageSelect
            id="target-lang"
            v-model="config.translation.target_lang"
            :invalid="!!issueText('translation.target_lang')"
          />
          <span v-if="issueText('translation.target_lang')" class="field-error">
            {{ issueText('translation.target_lang') }}
          </span>
        </div>
      </div>
      <p class="input-hint">{{ t('capture.languagesHint') }}</p>
    </section>

    <section class="pane-layout">
      <h3 class="pane-title">{{ t('capture.hotkeyTitle') }}</h3>
      <div class="grid-row">
        <div class="field-group">
          <label for="hotkey">{{ t('capture.hotkey') }}</label>
          <input
            id="hotkey"
            v-model="config.hotkeys"
            type="text"
            class="mac-input"
            spellcheck="false"
            autocomplete="off"
            :class="{ 'has-error': issueText('hotkeys') }"
            :placeholder="t('capture.hotkeyPlaceholder')"
          />
          <span v-if="issueText('hotkeys')" class="field-error" data-test="hotkey-error">
            {{ issueText('hotkeys') }}
          </span>
        </div>
        <div class="field-group">
          <label for="max-chars">{{ t('capture.maxChars') }}</label>
          <input
            id="max-chars"
            v-model.number="config.translation.max_chars"
            type="number"
            min="1"
            :max="MAX_CHARS_LIMIT"
            step="1"
            class="mac-input"
            :class="{ 'has-error': issueText('translation.max_chars') }"
          />
          <span v-if="issueText('translation.max_chars')" class="field-error">
            {{ issueText('translation.max_chars') }}
          </span>
          <span v-else class="input-hint">{{ t('capture.maxCharsHint') }}</span>
        </div>
      </div>
      <p class="input-hint">{{ t('capture.hotkeyHint') }}</p>
    </section>

    <section class="pane-layout">
      <h3 class="pane-title">{{ t('capture.providerTitle') }}</h3>
      <div class="grid-row">
        <div class="field-group">
          <label for="provider">{{ t('capture.provider') }}</label>
          <select id="provider" v-model="config.translation.provider" class="mac-select">
            <option v-for="provider in TRANSLATION_PROVIDERS" :key="provider" :value="provider">
              {{ t(PROVIDER_LABELS[provider]) }}
            </option>
          </select>
          <span v-if="issueText('translation.provider')" class="field-error">
            {{ issueText('translation.provider') }}
          </span>
        </div>
        <div class="field-group">
          <label for="contact-email">{{ t('capture.contactEmail') }}</label>
          <input
            id="contact-email"
            v-model="contactEmail"
            type="email"
            class="mac-input"
            autocomplete="email"
            spellcheck="false"
            data-test="contact-email"
            :class="{ 'has-error': issueText('translation.contact_email') }"
            :placeholder="t('capture.contactEmailPlaceholder')"
          />
          <span v-if="issueText('translation.contact_email')" class="field-error">
            {{ issueText('translation.contact_email') }}
          </span>
        </div>
      </div>
      <p class="input-hint">{{ t('capture.contactEmailHint') }}</p>
    </section>

    <section class="pane-layout">
      <h3 class="pane-title">{{ t('capture.tryTitle') }}</h3>
      <p class="input-hint">{{ t('capture.tryHint') }}</p>
      <div class="try-row">
        <input
          v-model="tryText"
          type="text"
          class="mac-input"
          data-test="try-text"
          :placeholder="t('capture.tryPlaceholder')"
          :aria-label="t('capture.tryPlaceholder')"
          @keydown.enter="onTryEnter"
        />
        <select
          v-model="tryProfile"
          class="mac-select try-profile"
          :aria-label="t('capture.tryProfile')"
        >
          <option value="">{{ t('capture.tryDefaultProfile') }}</option>
          <option v-for="profile in tryProfiles" :key="profile.id" :value="profile.id">
            {{ profile.label || profile.id }}
          </option>
        </select>
        <button
          type="button"
          class="btn btn-secondary"
          data-test="try-capture"
          :disabled="trying || !tryText.trim()"
          @click="tryCapture"
        >
          {{ t('capture.tryButton') }}
        </button>
      </div>
      <p v-if="tryResult" :class="['inline-result', tryResult.tone]">{{ tryResult.text }}</p>
    </section>
  </div>
</template>
