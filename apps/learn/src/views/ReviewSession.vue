<script setup lang="ts">
/**
 * The review session (Milestones L3 and I): today's queue, one card at a
 * time. The front comes first; revealing shows the meaning, pinyin, example
 * and audio, and the four rating buttons each say when FSRS would show the
 * card again (`previewRatings`). Space reveals, 1–4 rate. The summary at the
 * end shows the XP the pass earned and the streak.
 *
 * The meaning is labelled with the card's own `target_lang`: German
 * meanings are not bundled, so a German learner's new HSK words carry
 * English meanings (docs/architecture.md § 6.4), and the card says so. The
 * learner's meaning language is read again whenever the tab comes back,
 * since the settings tab can change it while a pass is open.
 *
 * Without the shortcuts (a screen reader in browse mode takes the number
 * keys) the card is worked with Tab and Enter, and focus follows the card
 * instead of dropping to the page as its buttons come and go: from "Show
 * answer" to the group of rating buttons (the group itself, so a second
 * Space never rates by accident), and from a rating (or a skip, or a retry)
 * to the next card's "Show answer", or to "Check for more cards" at the end.
 * The rating buttons stay focusable while a rating is saved (`aria-disabled`).
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AudioButton from '../components/AudioButton.vue'
import ProgressBar from '../components/ProgressBar.vue'
import { useReviewSession } from '../composables/useReviewSession'
import { usePanelActive, useShortcuts } from '../composables/useShortcuts'
import { useClock, usePlatform } from '../context'
import {
  INTERVAL_KEYS,
  RATINGS,
  RATING_KEYS,
  nextInterval,
  previewsByRating,
} from '../domain/intervals'
import { languageName } from '../domain/languages'
import type { LearnSettings, Rating } from '../platform/types'

const props = defineProps<{
  /** Bumped by the main window when today's goal changed: rebuild the queue. */
  round: number
  /** Today's quota of new characters, once known; an empty queue on a reviews-only day (0) offers to change it. */
  newGoal: number | null
}>()

const emit = defineEmits<{ 'change-goal': [] }>()

const { t, locale } = useI18n()
const platform = usePlatform()
const session = useReviewSession(platform, useClock())
const {
  phase,
  index,
  total,
  current,
  isNew,
  revealed,
  busy,
  error,
  xpEarned,
  rated,
  streak,
  goalReached,
} = session

const meaningLang = ref<LearnSettings['meaning_lang'] | null>(null)
const pane = ref<HTMLElement | null>(null)
const actions = ref<HTMLElement | null>(null)
const revealButton = ref<HTMLButtonElement | null>(null)
const ratingGroup = ref<HTMLElement | null>(null)
const moreButton = ref<HTMLButtonElement | null>(null)
/** A rating was given from the card's buttons: focus moves on with the card. */
let focusFollows = false

const card = computed(() => current.value?.capture.card ?? null)
const pinyin = computed(() => {
  const metadata = card.value?.script_metadata
  return metadata?.kind === 'chinese' && metadata.pinyin.trim() ? metadata.pinyin : null
})
const hskLabel = computed(() => {
  const metadata = card.value?.script_metadata
  if (metadata?.kind !== 'chinese' || metadata.hsk_level === null) return null
  return metadata.hsk_level >= 7
    ? t('review.hskBand')
    : t('review.hsk', { level: metadata.hsk_level })
})
const meaningFallback = computed(
  () =>
    current.value?.capture.origin === 'curriculum' &&
    meaningLang.value === 'de' &&
    card.value?.target_lang !== 'de',
)
/**
 * What "Play pronunciation" plays: the card's recording, else TTS of its
 * front. A platform without TTS has nothing to play for a card without a
 * recording, so it shows no button (views hide what the platform cannot do).
 */
const audioSource = computed(() => {
  if (!current.value || !card.value) return null
  if (!card.value.audio_front && !platform.capabilities.tts) return null
  return {
    cardId: current.value.cardId,
    inline: card.value.audio_front,
    text: card.value.front,
    lang: card.value.source_lang,
  }
})
const progressText = computed(() =>
  t('review.progress', { current: Math.min(index.value + 1, total.value), total: total.value }),
)

const numberFormat = computed(
  () => new Intl.NumberFormat(locale.value, { maximumFractionDigits: 1 }),
)

/** The rating buttons, each with the interval it would schedule (none without a preview). */
const ratingButtons = computed(() => {
  const previews = previewsByRating(current.value?.previews ?? [])
  const previewedAt = current.value?.previewedAt ?? 0
  return RATINGS.map((rating) => {
    const preview = previews.get(rating)
    const label = t(RATING_KEYS[rating])
    if (!preview) return { rating, label, interval: null, aria: label }
    const next = nextInterval(preview, previewedAt)
    const interval = t(INTERVAL_KEYS[next.unit], { n: numberFormat.value.format(next.value) })
    return { rating, label, interval, aria: t('review.ratingAria', { rating: label, interval }) }
  })
})

function focusIn(element: HTMLElement | null): boolean {
  const focused = document.activeElement
  return element !== null && focused !== null && element.contains(focused)
}

function reveal(): void {
  const fromButtons = focusIn(actions.value)
  session.reveal()
  if (fromButtons && revealed.value) void nextTick(() => ratingGroup.value?.focus())
}

function rate(rating: Rating): void {
  if (focusIn(actions.value)) focusFollows = true
  void session.rate(rating)
}

function skip(): void {
  if (focusIn(pane.value)) focusFollows = true
  void session.skip()
}

function retry(): void {
  if (focusIn(pane.value)) focusFollows = true
  void session.retry()
}

// The buttons that were used (a rating, skip, retry) are gone once the next
// card or the summary is shown; focus goes on to it, unless the learner
// moved it elsewhere meanwhile.
watch([current, phase], ([shown, now]) => {
  if (!focusFollows || now === 'loading' || (shown === null && now === 'card')) return
  focusFollows = false
  const focused = document.activeElement
  if (focused !== null && focused !== document.body && !focusIn(pane.value)) return
  void nextTick(() => {
    if (now === 'complete') moreButton.value?.focus()
    else if (now === 'card') revealButton.value?.focus()
  })
})

useShortcuts((key) => {
  if (phase.value !== 'card' || !current.value) return false
  if (key === ' ') {
    if (revealed.value) return false
    reveal()
    return true
  }
  if (!revealed.value || busy.value || !/^[1-4]$/.test(key)) return false
  rate(Number(key) as Rating)
  return true
})

async function loadMeaningLang(): Promise<void> {
  try {
    meaningLang.value = (await platform.getSettings()).meaning_lang
  } catch {
    meaningLang.value = null
  }
}

watch(
  () => props.round,
  () => void session.start(),
)

watch(
  usePanelActive(),
  (isActive) => {
    if (isActive) void loadMeaningLang()
  },
  { immediate: true },
)

onMounted(() => void session.start())
</script>

<template>
  <section ref="pane" class="pane-layout review-pane" data-test="review">
    <div class="pane-title-row">
      <h2 class="pane-title">{{ t('review.title') }}</h2>
      <span v-if="xpEarned > 0" class="badge badge-success" data-test="session-xp">
        {{ t('exercise.xp', { xp: xpEarned }) }}
      </span>
    </div>

    <p v-if="phase === 'loading'" class="input-hint spinner" data-test="review-loading">
      {{ t('review.loading') }}
    </p>

    <div v-else-if="phase === 'error'" class="onboarding-notice warning" data-test="review-error">
      <p role="alert">{{ t('review.loadFailed', { error: error?.message ?? '' }) }}</p>
      <button type="button" class="btn btn-primary" data-test="retry" @click="retry">
        {{ t('review.retry') }}
      </button>
    </div>

    <div v-else-if="phase === 'empty'" class="onboarding-notice" data-test="review-empty">
      <h3>{{ t('review.emptyTitle') }}</h3>
      <p>{{ t('review.emptyBody') }}</p>
      <p v-if="newGoal === 0">{{ t('review.emptyGoalHint') }}</p>
      <div class="action-row-center">
        <button
          v-if="newGoal === 0"
          type="button"
          class="btn btn-primary"
          data-test="change-goal"
          @click="emit('change-goal')"
        >
          {{ t('review.changeGoal') }}
        </button>
        <button type="button" class="btn btn-secondary" data-test="more" @click="session.start()">
          {{ t('review.more') }}
        </button>
      </div>
    </div>

    <div v-else-if="phase === 'complete'" class="onboarding-notice" data-test="review-complete">
      <h3>{{ t('review.completeTitle') }}</h3>
      <p v-if="goalReached" class="success-notice-card" data-test="goal-reached">
        {{ t('review.goalReached') }}
      </p>
      <dl class="stat-grid stat-grid-compact">
        <div class="stat-tile">
          <dt>{{ t('review.xpEarned') }}</dt>
          <dd data-test="summary-xp">{{ xpEarned }}</dd>
        </div>
        <div class="stat-tile">
          <dt>{{ t('review.reviewed') }}</dt>
          <dd data-test="summary-rated">{{ rated }}</dd>
        </div>
        <template v-if="streak">
          <div class="stat-tile">
            <dt>{{ t('review.streakCurrent') }}</dt>
            <dd data-test="summary-streak">{{ streak.current }}</dd>
          </div>
          <div class="stat-tile">
            <dt>{{ t('review.streakLongest') }}</dt>
            <dd data-test="summary-longest">{{ streak.longest }}</dd>
          </div>
        </template>
      </dl>
      <button
        ref="moreButton"
        type="button"
        class="btn btn-secondary"
        data-test="more"
        @click="session.start()"
      >
        {{ t('review.more') }}
      </button>
    </div>

    <template v-else>
      <ProgressBar
        :value="index"
        :max="total"
        :label="t('review.progressLabel')"
        :text="progressText"
      />

      <article class="flashcard" data-test="card" :data-card-id="current?.cardId">
        <p v-if="!current && busy" class="input-hint spinner">{{ t('review.loadingCard') }}</p>
        <template v-else-if="current && card">
          <div class="flashcard-badges">
            <span v-if="isNew" class="badge" data-test="new-badge">{{ t('review.newWord') }}</span>
            <span v-if="hskLabel" class="badge badge-muted" data-test="hsk">{{ hskLabel }}</span>
          </div>
          <p class="flashcard-front" :lang="card.source_lang" data-test="front">
            {{ card.front }}
          </p>
          <div class="flashcard-answer" aria-live="polite" data-test="answer">
            <template v-if="revealed">
              <p v-if="pinyin" class="flashcard-pinyin" data-test="pinyin">{{ pinyin }}</p>
              <div class="flashcard-field">
                <span class="field-label">{{
                  t('review.meaning', { language: languageName(card.target_lang, t) })
                }}</span>
                <p class="flashcard-back" :lang="card.target_lang" data-test="back">
                  {{ card.back }}
                </p>
                <p v-if="meaningFallback" class="input-hint" data-test="meaning-fallback">
                  {{ t('review.meaningFallback') }}
                </p>
              </div>
              <div v-if="card.example_source.trim()" class="flashcard-field" data-test="example">
                <span class="field-label">{{ t('review.example') }}</span>
                <p :lang="card.source_lang">{{ card.example_source }}</p>
                <p
                  v-if="card.example_target.trim()"
                  class="flashcard-example-target"
                  :lang="card.target_lang"
                >
                  {{ card.example_target }}
                </p>
              </div>
              <div v-if="card.notes.trim()" class="flashcard-field" data-test="notes">
                <span class="field-label">{{ t('review.notes') }}</span>
                <p>{{ card.notes }}</p>
              </div>
              <AudioButton
                v-if="audioSource"
                :key="current.cardId"
                :source="audioSource"
                :label="t('review.playAudio')"
              />
            </template>
          </div>
        </template>
      </article>

      <div v-if="error" class="notice error toast-row" role="alert" data-test="review-card-error">
        <span>{{
          error.step === 'rate'
            ? t('review.rateFailed', { error: error.message })
            : t('review.cardFailed', { error: error.message })
        }}</span>
        <span class="toolbar">
          <button
            v-if="error.step === 'card'"
            type="button"
            class="btn btn-secondary btn-mini"
            data-test="retry"
            @click="retry"
          >
            {{ t('review.retry') }}
          </button>
          <button type="button" class="btn btn-secondary btn-mini" data-test="skip" @click="skip">
            {{ t('review.skip') }}
          </button>
        </span>
      </div>

      <div v-if="current" ref="actions" class="review-actions">
        <button
          v-if="!revealed"
          ref="revealButton"
          type="button"
          class="btn btn-primary btn-large"
          data-test="reveal"
          @click="reveal"
        >
          {{ t('review.showAnswer') }} <kbd aria-hidden="true">{{ t('review.spaceKey') }}</kbd>
        </button>
        <div
          v-else
          ref="ratingGroup"
          class="rating-row"
          role="group"
          tabindex="-1"
          :aria-label="t('review.rateLabel')"
          data-test="rating-group"
        >
          <button
            v-for="button in ratingButtons"
            :key="button.rating"
            type="button"
            class="rating-btn"
            :class="`rating-${button.rating}`"
            :aria-disabled="busy || undefined"
            :aria-label="button.aria"
            :data-test="`rate-${button.rating}`"
            @click="rate(button.rating)"
          >
            <span class="rating-name">{{ button.label }}</span>
            <span v-if="button.interval" class="rating-interval" data-test="interval">{{
              button.interval
            }}</span>
            <kbd aria-hidden="true">{{ button.rating }}</kbd>
          </button>
        </div>
        <p class="input-hint">{{ t('review.keysHint') }}</p>
      </div>
    </template>
  </section>
</template>
