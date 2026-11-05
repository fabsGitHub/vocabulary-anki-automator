<script setup lang="ts">
/**
 * Practice (Milestones M2/M3): pick a learned word and an exercise type, get
 * an exercise from `nextExercise` with a fresh random seed, answer it, ask
 * for the next one.
 *
 * Quiz isolation (M1) is the core's guarantee: the words offered here are
 * `listLearnedCards` (the learned pool), and an exercise shows exactly the
 * texts the adapter generated, nothing looked up on the side. When the pool
 * cannot make an exercise of the chosen type for a word (too few learned
 * words, no learned sentence using it), the adapter's reason is shown.
 *
 * An exercise that arrives after the word or the type was changed is
 * dropped, so the exercise on screen is always one for the current choice.
 * Keyboard focus is kept on the generate button: it is not disabled while an
 * exercise is prepared, and once an exercise is answered (which disables its
 * controls, and the browser drops their focus) focus moves to it, the
 * "Next exercise" a keyboard user goes on with.
 */
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import ListeningChoice from '../components/exercises/ListeningChoice.vue'
import ListeningSentence from '../components/exercises/ListeningSentence.vue'
import MultipleChoice from '../components/exercises/MultipleChoice.vue'
import SentenceBuilder from '../components/exercises/SentenceBuilder.vue'
import { usePanelActive } from '../composables/useShortcuts'
import { usePlatform, useSeeds } from '../context'
import { errorMessage } from '../domain/errors'
import { EXERCISE_KINDS, KIND_KEYS, unavailableReason } from '../domain/exercises'
import { languageName } from '../domain/languages'
import type { Exercise, ExerciseKind, LearnedCard } from '../platform/types'

const { t } = useI18n()
const platform = usePlatform()
const seeds = useSeeds()
const active = usePanelActive()

const cards = ref<LearnedCard[]>([])
const loading = ref(false)
const loaded = ref(false)
const loadError = ref<string | null>(null)
const cardId = ref<number | null>(null)
const kind = ref<ExerciseKind>('multiple_choice')
const exercise = ref<Exercise | null>(null)
/** Remounts the exercise component, so each exercise starts unanswered. */
const exerciseKey = ref(0)
const generating = ref(false)
const generateError = ref<string | null>(null)
const answered = ref(false)
const generateButton = ref<HTMLButtonElement | null>(null)
const exerciseFrame = ref<HTMLElement | null>(null)
/** Numbers the exercise requests; only the latest one may show its answer. */
let request = 0

const selectedCard = computed(
  () => cards.value.find((card) => card.card_id === cardId.value) ?? null,
)
const kinds = computed(() =>
  EXERCISE_KINDS.map((value) => {
    const reason = unavailableReason(value, selectedCard.value, platform.capabilities)
    return { value, label: t(KIND_KEYS[value]), reason: reason ? t(reason) : null }
  }),
)
const kindAvailable = computed(
  () => !kinds.value.find((entry) => entry.value === kind.value)?.reason,
)
/** An exercise waits for its answer: the button only skips it, so it steps back for "Check". */
const skipping = computed(() => exercise.value !== null && !answered.value)
const generateLabel = computed(() => {
  if (generating.value) return t('practice.generating')
  if (exercise.value === null) return t('practice.start')
  return skipping.value ? t('practice.skip') : t('practice.next')
})

function cardLabel(card: LearnedCard): string {
  return `${card.front} · ${languageName(card.source_lang, t)} → ${languageName(card.target_lang, t)}`
}

async function loadCards(): Promise<void> {
  if (loading.value) return
  loading.value = true
  loadError.value = null
  try {
    const learned = await platform.listLearnedCards()
    cards.value = learned
    if (!learned.some((card) => card.card_id === cardId.value)) {
      cardId.value = learned[0]?.card_id ?? null
    }
    loaded.value = true
  } catch (err) {
    loadError.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

async function generate(): Promise<void> {
  if (generating.value || cardId.value === null || !kindAvailable.value) return
  const asked = ++request
  generating.value = true
  generateError.value = null
  try {
    const next = await platform.nextExercise(cardId.value, kind.value, seeds.next())
    if (asked !== request) return
    exercise.value = next
    answered.value = false
    exerciseKey.value += 1
  } catch (err) {
    if (asked !== request) return
    exercise.value = null
    generateError.value = errorMessage(err)
  } finally {
    if (asked === request) generating.value = false
  }
}

function onAnswered(): void {
  answered.value = true
  const focused = document.activeElement
  const focusLost = focused === null || focused === document.body
  if (focusLost || exerciseFrame.value?.contains(focused)) {
    void nextTick(() => generateButton.value?.focus())
  }
}

// A new word or type makes the exercise on screen (or on its way) a stale one.
watch([cardId, kind], () => {
  request += 1
  generating.value = false
  exercise.value = null
  generateError.value = null
  answered.value = false
})

// Words learned in a review since the last visit show up when the tab comes back.
watch(
  active,
  (isActive) => {
    if (isActive) void loadCards()
  },
  { immediate: true },
)
</script>

<template>
  <section class="pane-layout practice-pane" data-test="practice">
    <div>
      <h2 class="pane-title">{{ t('practice.title') }}</h2>
      <p class="input-hint">{{ t('practice.intro') }}</p>
    </div>

    <p v-if="loadError" class="notice error" role="alert" data-test="practice-error">
      {{ t('practice.loadFailed', { error: loadError }) }}
    </p>
    <p v-if="!loaded && loading" class="input-hint spinner">{{ t('practice.loading') }}</p>
    <p v-else-if="loaded && cards.length === 0" class="empty-state" data-test="practice-empty">
      {{ t('practice.empty') }}
    </p>

    <template v-else-if="loaded">
      <div class="practice-controls">
        <div class="field-group">
          <label for="practice-card">{{ t('practice.card') }}</label>
          <select id="practice-card" v-model="cardId" class="mac-select" data-test="practice-card">
            <option v-for="card in cards" :key="card.card_id" :value="card.card_id">
              {{ cardLabel(card) }}
            </option>
          </select>
        </div>

        <fieldset class="kind-picker">
          <legend class="field-label">{{ t('practice.kind') }}</legend>
          <label
            v-for="entry in kinds"
            :key="entry.value"
            class="kind-option"
            :class="{ 'is-selected': kind === entry.value, 'is-disabled': entry.reason }"
            :title="entry.reason ?? undefined"
          >
            <input
              v-model="kind"
              type="radio"
              name="practice-kind"
              :value="entry.value"
              :disabled="Boolean(entry.reason)"
              :data-test="`kind-${entry.value}`"
            />
            <span>{{ entry.label }}</span>
          </label>
        </fieldset>
        <p
          v-for="entry in kinds.filter((option) => option.reason && option.value === kind)"
          :key="entry.value"
          class="input-hint"
        >
          {{ entry.reason }}
        </p>
      </div>

      <div class="action-row">
        <button
          ref="generateButton"
          type="button"
          class="btn"
          :class="skipping ? 'btn-secondary' : 'btn-primary'"
          data-test="generate"
          :disabled="cardId === null || !kindAvailable"
          :aria-disabled="generating || undefined"
          :aria-busy="generating || undefined"
          @click="generate"
        >
          {{ generateLabel }}
        </button>
      </div>

      <p v-if="generateError" class="notice warning" role="alert" data-test="generate-error">
        {{ t('practice.generateFailed', { error: generateError }) }}
      </p>

      <div v-if="exercise" ref="exerciseFrame" class="exercise-frame" data-test="exercise">
        <MultipleChoice
          v-if="exercise.kind === 'multiple_choice'"
          :key="exerciseKey"
          :exercise="exercise"
          @answered="onAnswered"
        />
        <SentenceBuilder
          v-else-if="exercise.kind === 'sentence_builder'"
          :key="exerciseKey"
          :exercise="exercise"
          @answered="onAnswered"
        />
        <ListeningChoice
          v-else-if="exercise.kind === 'listening_choice'"
          :key="exerciseKey"
          :exercise="exercise"
          @answered="onAnswered"
        />
        <ListeningSentence v-else :key="exerciseKey" :exercise="exercise" @answered="onAnswered" />
      </div>
    </template>
  </section>
</template>
