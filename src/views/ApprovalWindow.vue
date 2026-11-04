<script setup lang="ts">
/**
 * The `preview` window (Milestone D): review the captured card, fix what the
 * providers got wrong or could not fetch, then add it (Enter) or discard it
 * (Esc). The capture flow and its race handling live in `useApproval`; this
 * component adds what is about the window itself: keys, audio, focus, and
 * the UI language, re-read per capture because this webview never sees the
 * main window's language switch. A detected source language shows as chips,
 * one per language the text could be in, with a hint when the guess is unsure;
 * a click on another chip captures the card again in that language.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { getCurrentSettings, type AppConfig, type CardField, type PendingCapture } from '../api'
import { useApproval } from '../composables/useApproval'
import { isSupportedLang } from '../domain/language'
import { formatWarning } from '../domain/warnings'
import { LANGUAGE_NAME_KEYS, setLocale } from '../i18n'

const { t, te } = useI18n()
const approval = useApproval()
const {
  pending,
  draft,
  phase,
  result,
  error,
  captureError,
  fieldCheck,
  switchingTo,
  chinese,
  frontEdited,
} = approval

const settings = ref<AppConfig | null>(null)
const backInput = ref<HTMLInputElement | null>(null)
const chipGroup = ref<HTMLElement | null>(null)
const editable = computed(() => phase.value === 'editing')

const warnings = computed(() =>
  (pending.value?.warnings ?? []).map((warning) =>
    formatWarning(
      warning,
      (key, params) => t(key, params ?? {}),
      (key) => te(key),
    ),
  ),
)

function languageName(code: string): string {
  return isSupportedLang(code) ? t(LANGUAGE_NAME_KEYS[code]) : code
}

const languagePair = computed(() => {
  const card = pending.value?.card
  return card
    ? t('approval.languagePair', {
        source: languageName(card.source_lang),
        target: languageName(card.target_lang),
      })
    : ''
})

const detection = computed(() => pending.value?.detection ?? null)

interface LanguageChip {
  code: string
  active: boolean
  label: string
  classes: string
}

/**
 * One chip per language the text could be in; none when there is nothing to
 * switch to. While switching, the chosen language is the active chip.
 */
const languageChips = computed<LanguageChip[]>(() => {
  const found = detection.value
  if (!found || found.candidates.length < 2) return []
  const target = switchingTo.value
  return found.candidates.map((code) => {
    const name = languageName(code)
    if (target !== null) {
      return code === target
        ? { code, active: true, label: t('approval.switching', { lang: name }), classes: 'badge' }
        : { code, active: false, label: name, classes: 'badge badge-muted' }
    }
    const active = code === found.lang
    if (!active) return { code, active, label: name, classes: 'badge badge-muted' }
    return found.confident
      ? { code, active, label: name, classes: 'badge' }
      : {
          code,
          active,
          label: t('approval.detectedUnsure', { lang: name }),
          classes: 'badge badge-warning',
        }
  })
})

/** Why the detected language may be wrong; `null` when it is certain or being switched. */
const languageHint = computed(() => {
  const found = detection.value
  if (!found || found.confident || phase.value === 'switching') return null
  const lang = languageName(found.lang)
  if (found.method !== 'fallback') return t('approval.unsureHint', { lang })
  return found.script === 'latin' || found.script === 'none'
    ? t('approval.fallbackHint', { lang })
    : t('approval.foreignScript', { lang })
})

/**
 * Captures the card again as `code`; afterwards focus goes to the new active
 * chip, unless the back is empty (then the back field has it). After a failed
 * switch it goes back to the chip that was clicked, which lost it while disabled.
 */
async function switchTo(code: string): Promise<void> {
  const shown = await approval.switchLanguage(code, languageName(code))
  if (shown && !pending.value?.card.back.trim()) return
  await nextTick()
  if (!shown && document.activeElement && document.activeElement !== document.body) return
  const selector = shown ? 'button[aria-pressed="true"]' : `button[data-test="language-${code}"]`
  chipGroup.value?.querySelector<HTMLButtonElement>(selector)?.focus()
}

const profile = computed(() => {
  const id = pending.value?.profile_id
  return id ? (settings.value?.hotkey_profiles.find((p) => p.id === id) ?? null) : null
})

function valueName(source: CardField): string {
  return t(`mapping.sources.${source}`)
}

/** One sentence per value the pending card's note type would drop. */
const lostValues = computed(() => {
  const check = fieldCheck.value
  if (check?.status !== 'checked') return []
  const model = check.model
  return check.lost.map((lost) => {
    switch (lost.kind) {
      case 'missing_field':
        return t('approval.lostMissingField', {
          value: valueName(lost.source),
          model,
          field: lost.anki_field,
        })
      case 'unmapped':
        return t('approval.lostUnmapped', { value: valueName(lost.source), model })
      case 'audio':
        return lost.audio_field
          ? t('approval.lostAudio', { model, field: lost.audio_field })
          : t('approval.lostAudioUnset', { model })
    }
  })
})

const fieldCheckError = computed(() =>
  fieldCheck.value?.status === 'failed' ? fieldCheck.value.error : null,
)

const hskLabel = computed(() => {
  const level = chinese.value?.hsk_level ?? null
  if (level === null) return t('approval.notInHsk')
  return level >= 7 ? t('approval.hskBand') : t('approval.hsk', { level })
})

const audioSrc = computed(() => {
  const audio = pending.value?.card.audio_front
  return audio ? `data:audio/mpeg;base64,${audio}` : null
})

let player: HTMLAudioElement | null = null

/** Plays the TTS of the captured front; the click is the user gesture autoplay rules ask for. */
function playAudio(): void {
  if (!audioSrc.value) return
  player?.pause()
  player = new Audio(audioSrc.value)
  const failed = (err: unknown) => console.warn('[approval] audio playback failed:', err)
  try {
    player.play()?.catch(failed)
  } catch (err) {
    failed(err)
  }
}

async function refreshSettings(): Promise<void> {
  try {
    const loaded = await getCurrentSettings()
    settings.value = loaded
    setLocale(loaded.ui_lang)
  } catch (err) {
    console.warn('[approval] could not read the settings:', err)
  }
  document.title = t('approval.windowTitle')
}

/** Enter adds and Esc discards from anywhere in the window, except where a key means something else. */
function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing || event.keyCode === 229) return
  if (event.key === 'Escape') {
    event.preventDefault()
    void (phase.value === 'saved' ? approval.close() : approval.discard())
    return
  }
  if (event.key !== 'Enter' || event.shiftKey || event.altKey) return
  const target = event.target
  if (target instanceof HTMLButtonElement || target instanceof HTMLAnchorElement) return
  if (target instanceof HTMLElement && target.tagName === 'SUMMARY') return
  event.preventDefault()
  void (phase.value === 'saved' ? approval.close() : approval.approve())
}

watch(pending, (capture: PendingCapture | null, previous: PendingCapture | null) => {
  if (!capture || capture === previous) return
  void refreshSettings()
  if (!capture.card.back.trim()) void nextTick(() => backInput.value?.focus())
})

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  void refreshSettings()
  void approval.start()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  player?.pause()
})
</script>

<template>
  <main class="view approval-view app-fade-in">
    <div v-if="captureError" class="notice error toast-row" role="alert" data-test="capture-error">
      <span>{{ t('app.captureFailed', { message: captureError }) }}</span>
      <button
        type="button"
        class="close-btn"
        :aria-label="t('app.dismiss')"
        @click="approval.dismissCaptureError()"
      >
        &times;
      </button>
    </div>

    <section v-if="phase === 'waiting'" class="onboarding-notice" data-test="waiting">
      <p>{{ t('approval.waiting') }}</p>
    </section>

    <div v-else class="card approval-card" data-test="approval-card">
      <header class="approval-header">
        <h2>{{ t('approval.windowTitle') }}</h2>
        <div class="approval-meta">
          <template v-if="languageChips.length > 0 && pending">
            <div
              ref="chipGroup"
              class="language-chips"
              role="group"
              :aria-label="t('approval.languageGroup')"
              data-test="language-chips"
            >
              <button
                v-for="chip in languageChips"
                :key="chip.code"
                type="button"
                :class="chip.classes"
                :aria-pressed="chip.active ? 'true' : 'false'"
                :title="t('approval.switchTo', { lang: languageName(chip.code) })"
                :aria-label="t('approval.switchTo', { lang: languageName(chip.code) })"
                :data-test="`language-${chip.code}`"
                :disabled="phase !== 'editing'"
                @click="switchTo(chip.code)"
              >
                {{ chip.label }}
              </button>
            </div>
            <span class="badge badge-muted" data-test="target-lang">{{
              t('approval.targetLang', { lang: languageName(pending.card.target_lang) })
            }}</span>
          </template>
          <span v-else class="badge badge-muted" data-test="language-pair">{{ languagePair }}</span>
          <span v-if="profile" class="badge badge-muted">{{
            t('approval.profile', { label: profile.label })
          }}</span>
          <span v-if="pending?.filing.deck" class="badge badge-muted" data-test="deck">{{
            t('approval.deck', { deck: pending.filing.deck })
          }}</span>
          <span v-if="pending?.filing.model" class="badge badge-muted" data-test="model">{{
            t('approval.model', { model: pending.filing.model })
          }}</span>
        </div>
      </header>

      <p v-if="switchingTo" class="input-hint" role="status" data-test="switching">
        {{ t('approval.switching', { lang: languageName(switchingTo) }) }}
      </p>
      <p v-if="languageHint" class="notice warning" role="status" data-test="language-hint">
        {{ languageHint }}
      </p>

      <div v-if="warnings.length > 0" class="notice warning" data-test="warnings">
        <strong>{{ t('warnings.title') }}</strong>
        <ul class="notice-list">
          <li v-for="(warning, index) in warnings" :key="index" data-test="warning">
            {{ warning.text }}
            <small v-if="warning.detail" class="notice-detail">{{ warning.detail }}</small>
          </li>
        </ul>
      </div>

      <div class="grid-row">
        <div class="field-group">
          <label for="card-front">{{ t('approval.front') }}</label>
          <div class="input-with-action">
            <input
              id="card-front"
              v-model="draft.front"
              type="text"
              class="mac-input"
              data-test="front"
              :readonly="!editable"
            />
            <button
              v-if="audioSrc"
              type="button"
              class="btn btn-secondary btn-icon"
              data-test="play-audio"
              :aria-label="t('approval.playAudio')"
              :title="t('approval.playAudio')"
              @click="playAudio"
            >
              ▶
            </button>
          </div>
        </div>
        <div class="field-group">
          <label for="card-back">{{ t('approval.back') }}</label>
          <input
            id="card-back"
            ref="backInput"
            v-model="draft.back"
            type="text"
            class="mac-input"
            data-test="back"
            :readonly="!editable"
          />
        </div>
      </div>
      <p v-if="frontEdited && audioSrc" class="input-hint">{{ t('approval.audioStale') }}</p>

      <div v-if="chinese" class="chinese-row" data-test="chinese">
        <div class="field-group">
          <span class="field-label">{{ t('approval.pinyin') }}</span>
          <span class="readonly-value" lang="zh-Latn" data-test="pinyin">{{ chinese.pinyin }}</span>
        </div>
        <span class="badge" data-test="hsk">{{ hskLabel }}</span>
      </div>

      <div class="field-group">
        <label for="card-example-source">{{ t('approval.exampleSource') }}</label>
        <textarea
          id="card-example-source"
          v-model="draft.example_source"
          rows="2"
          data-test="example-source"
          :readonly="!editable"
        />
      </div>
      <div class="field-group">
        <label for="card-example-target">{{ t('approval.exampleTarget') }}</label>
        <textarea
          id="card-example-target"
          v-model="draft.example_target"
          rows="2"
          data-test="example-target"
          :readonly="!editable"
        />
      </div>
      <div class="field-group">
        <label for="card-notes">{{ t('approval.notes') }}</label>
        <input
          id="card-notes"
          v-model="draft.notes"
          type="text"
          class="mac-input"
          data-test="notes"
          :readonly="!editable"
        />
      </div>

      <div
        v-if="lostValues.length > 0 || fieldCheckError"
        class="notice warning"
        role="status"
        data-test="fields-lost"
      >
        <p v-if="fieldCheckError">
          {{ t('approval.fieldCheckFailed', { error: fieldCheckError }) }}
        </p>
        <template v-else>
          <strong>{{ t('approval.lostTitle') }}</strong>
          <ul class="notice-list">
            <li v-for="(line, index) in lostValues" :key="index" data-test="lost-value">
              {{ line }}
            </li>
          </ul>
          <p class="notice-detail">{{ t('approval.lostHint') }}</p>
        </template>
      </div>

      <div v-if="phase === 'saved'" class="notice warning" role="status" data-test="saved-offline">
        <p>{{ t('approval.savedOffline') }}</p>
        <p v-if="result?.anki_error" class="notice-detail">
          {{ t('approval.ankiError', { error: result.anki_error }) }}
        </p>
      </div>
      <p v-if="error" class="notice error" role="alert" data-test="approval-error">
        {{ t(error.key, error.params ?? {}) }}
      </p>

      <footer class="action-row approval-actions">
        <span class="input-hint">{{ t('approval.keyboardHint') }}</span>
        <button
          v-if="phase === 'saved'"
          type="button"
          class="btn btn-primary"
          data-test="close"
          @click="approval.close()"
        >
          {{ t('approval.close') }}
        </button>
        <template v-else>
          <button
            type="button"
            class="btn btn-secondary"
            data-test="discard"
            :disabled="phase === 'submitting' || phase === 'switching'"
            @click="approval.discard()"
          >
            {{ t('approval.discard') }}
          </button>
          <button
            type="button"
            class="btn btn-primary"
            data-test="approve"
            :disabled="phase !== 'editing' || !draft.front.trim()"
            @click="approval.approve()"
          >
            {{ phase === 'submitting' ? t('approval.approving') : t('approval.approve') }}
          </button>
        </template>
      </footer>
    </div>
  </main>
</template>
