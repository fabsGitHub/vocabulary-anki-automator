<script setup lang="ts">
/**
 * Contextual lessons (Milestone I, Babbel-style): short runs of the
 * learner's own captures. Each word is shown in the sentence it was
 * captured with, then that sentence comes back as a cloze whose gap can be
 * revealed. Reaching the end completes the lesson (`completeLesson`), which
 * awards XP at most once per lesson and day; the adapter says how much.
 *
 * Titles are built here from the lesson's languages and its number within
 * its language pair: `Lesson.title` is only the core's English fallback.
 *
 * Stepping through a lesson by keyboard never drops focus to the page: one
 * forward button turns from "Next" into "Complete lesson" at the last step,
 * the buttons that are unavailable for the moment are `aria-disabled` rather
 * than disabled, and revealing a gap hands focus from the reveal button
 * (which goes away) to the forward button. The revealed sentence is read out.
 */
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import ProgressBar from '../components/ProgressBar.vue'
import { usePanelActive } from '../composables/useShortcuts'
import { usePlatform } from '../context'
import { errorMessage } from '../domain/errors'
import { languageName } from '../domain/languages'
import { clozeParts, lessonNumbers, lessonSteps } from '../domain/lessons'
import type { Lesson } from '../platform/types'

const { t } = useI18n()
const platform = usePlatform()
const active = usePanelActive()

const lessons = ref<Lesson[]>([])
const loading = ref(false)
const loaded = ref(false)
const loadError = ref<string | null>(null)
const lesson = ref<Lesson | null>(null)
/** The open lesson's number within its language pair. */
const lessonNumber = ref(0)
const stepIndex = ref(0)
const revealed = ref(false)
const completing = ref(false)
const completedXp = ref<number | null>(null)
const completeError = ref<string | null>(null)
const revealButton = ref<HTMLButtonElement | null>(null)
const forwardButton = ref<HTMLButtonElement | null>(null)

const titles = computed(() => {
  const numbers = lessonNumbers(lessons.value)
  return lessons.value.map((entry, index) => lessonTitle(entry, numbers[index]))
})
const activeTitle = computed(() =>
  lesson.value ? lessonTitle(lesson.value, lessonNumber.value) : '',
)
const steps = computed(() => (lesson.value ? lessonSteps(lesson.value) : []))
const step = computed(() => steps.value[stepIndex.value] ?? null)
const isLast = computed(() => stepIndex.value === steps.value.length - 1)
const cloze = computed(() => (step.value?.kind === 'cloze' ? clozeParts(step.value.card) : null))
const progressText = computed(() =>
  t('lessons.step', { current: stepIndex.value + 1, total: steps.value.length }),
)
/** The forward button does nothing more for this lesson: it is being, or has been, completed. */
const forwardDone = computed(() => isLast.value && (completing.value || completedXp.value !== null))
const forwardLabel = computed(() => {
  if (!isLast.value) return t('lessons.next')
  return completing.value ? t('lessons.completing') : t('lessons.complete')
})

function lessonTitle(entry: Lesson, number: number): string {
  return t('lessons.lessonTitle', {
    source: languageName(entry.source_lang, t),
    target: languageName(entry.target_lang, t),
    number,
  })
}

async function load(): Promise<void> {
  if (loading.value) return
  loading.value = true
  loadError.value = null
  try {
    lessons.value = await platform.listLessons()
    loaded.value = true
  } catch (err) {
    loadError.value = errorMessage(err)
  } finally {
    loading.value = false
  }
}

function open(index: number): void {
  lesson.value = lessons.value[index]
  lessonNumber.value = lessonNumbers(lessons.value)[index]
  stepIndex.value = 0
  revealed.value = false
  completedXp.value = null
  completeError.value = null
}

function close(): void {
  lesson.value = null
}

function go(delta: number): void {
  const next = stepIndex.value + delta
  if (next < 0 || next >= steps.value.length) return
  stepIndex.value = next
  revealed.value = false
}

function forward(): void {
  if (isLast.value) void complete()
  else go(1)
}

function reveal(): void {
  const hadFocus = revealButton.value !== null && document.activeElement === revealButton.value
  revealed.value = true
  if (hadFocus) void nextTick(() => forwardButton.value?.focus())
}

async function complete(): Promise<void> {
  if (!lesson.value || completing.value || completedXp.value !== null) return
  completing.value = true
  completeError.value = null
  try {
    completedXp.value = await platform.completeLesson(lesson.value.id)
  } catch (err) {
    completeError.value = errorMessage(err)
  } finally {
    completing.value = false
  }
}

// New captures can make new lessons; the list refreshes whenever the tab comes back.
watch(
  active,
  (isActive) => {
    if (isActive && !lesson.value) void load()
  },
  { immediate: true },
)
</script>

<template>
  <section class="pane-layout lessons-pane" data-test="lessons">
    <template v-if="!lesson">
      <div>
        <h2 class="pane-title">{{ t('lessons.title') }}</h2>
        <p class="input-hint">{{ t('lessons.intro') }}</p>
      </div>
      <p v-if="loadError" class="notice error" role="alert" data-test="lessons-error">
        {{ t('lessons.loadFailed', { error: loadError }) }}
      </p>
      <p v-if="!loaded && loading" class="input-hint spinner">{{ t('lessons.loading') }}</p>
      <p v-else-if="loaded && lessons.length === 0" class="empty-state" data-test="lessons-empty">
        {{ t('lessons.empty') }}
      </p>
      <ul v-else class="lesson-list">
        <li
          v-for="(entry, index) in lessons"
          :key="entry.id"
          class="lesson-item"
          data-test="lesson"
        >
          <div>
            <h3 class="lesson-title" data-test="lesson-title">{{ titles[index] }}</h3>
            <p class="input-hint">{{ t('lessons.cardCount', { count: entry.cards.length }) }}</p>
          </div>
          <button
            type="button"
            class="btn btn-primary btn-mini"
            :data-test="`open-${entry.id}`"
            :aria-label="t('lessons.startLesson', { title: titles[index] })"
            @click="open(index)"
          >
            {{ t('lessons.start') }}
          </button>
        </li>
      </ul>
    </template>

    <template v-else>
      <div class="pane-title-row">
        <h2 class="pane-title" data-test="active-lesson-title">{{ activeTitle }}</h2>
        <button type="button" class="btn btn-secondary btn-mini" data-test="close" @click="close">
          {{ t('lessons.back') }}
        </button>
      </div>
      <ProgressBar
        :value="stepIndex + 1"
        :max="steps.length"
        :label="t('lessons.progressLabel')"
        :text="progressText"
      />

      <article
        v-if="step"
        class="flashcard lesson-card"
        :data-test="`step-${step.kind}`"
        :data-card-id="step.card.card_id"
      >
        <template v-if="step.kind === 'context'">
          <span class="field-label">{{ t('lessons.context') }}</span>
          <p class="flashcard-front" :lang="lesson.source_lang" data-test="word">
            {{ step.card.word }}
          </p>
          <p v-if="step.card.pinyin" class="flashcard-pinyin">{{ step.card.pinyin }}</p>
          <p v-if="step.card.translation" class="flashcard-back" :lang="lesson.target_lang">
            {{ step.card.translation }}
          </p>
          <div class="flashcard-field">
            <p class="lesson-sentence" :lang="lesson.source_lang" data-test="sentence-source">
              {{ step.card.sentence_source }}
            </p>
            <p
              class="flashcard-example-target"
              :lang="lesson.target_lang"
              data-test="sentence-target"
            >
              {{ step.card.sentence_target }}
            </p>
          </div>
        </template>

        <template v-else>
          <span class="field-label">{{ t('lessons.cloze') }}</span>
          <p class="input-hint">{{ t('lessons.clozeHint') }}</p>
          <p
            class="lesson-sentence lesson-cloze"
            :lang="lesson.source_lang"
            aria-live="polite"
            aria-atomic="true"
            data-test="cloze"
          >
            <template v-if="cloze">
              {{ cloze.before
              }}<mark v-if="revealed" class="cloze-word" data-test="cloze-word">{{
                cloze.gap
              }}</mark
              ><span v-else class="cloze-gap" role="img" :aria-label="t('lessons.blank')"></span
              >{{ cloze.after }}
            </template>
            <template v-else>{{ step.card.sentence_source }}</template>
          </p>
          <p class="flashcard-example-target" :lang="lesson.target_lang">
            {{ step.card.sentence_target }}
          </p>
          <div aria-live="polite">
            <p v-if="revealed && step.card.pinyin" class="flashcard-pinyin">
              {{ step.card.pinyin }}
            </p>
          </div>
          <button
            v-if="!revealed && cloze"
            ref="revealButton"
            type="button"
            class="btn btn-secondary"
            data-test="reveal"
            @click="reveal"
          >
            {{ t('lessons.reveal') }}
          </button>
        </template>
      </article>

      <div class="action-row">
        <button
          type="button"
          class="btn btn-secondary"
          data-test="previous"
          :aria-disabled="stepIndex === 0 || undefined"
          @click="go(-1)"
        >
          {{ t('lessons.previous') }}
        </button>
        <button
          ref="forwardButton"
          type="button"
          class="btn btn-primary"
          :data-test="isLast ? 'complete' : 'next'"
          :aria-disabled="forwardDone || undefined"
          @click="forward"
        >
          {{ forwardLabel }}
        </button>
      </div>

      <div role="status" data-test="lesson-result">
        <p v-if="completedXp !== null && completedXp > 0" class="success-notice-card">
          {{ t('lessons.completed', { xp: completedXp }) }}
        </p>
        <p v-else-if="completedXp !== null" class="notice">{{ t('lessons.completedNoXp') }}</p>
      </div>
      <p v-if="completeError" class="notice error" role="alert" data-test="complete-error">
        {{ t('lessons.completeFailed', { error: completeError }) }}
      </p>
    </template>
  </section>
</template>
