<script setup lang="ts">
/**
 * The first-run setup assistant (`useSetupAssistant` holds its logic). The
 * main window shows it instead of the tabs until it is finished or dismissed,
 * and the header's "Setup assistant" button brings it back.
 *
 * It uses the main window's Anki connection and settings store, so the header
 * badge, the Anki tab and the assistant always agree, and an install started
 * here cannot overlap one started there. Installing waits for a click and the
 * consent dialog: "Set up everything" asks once for every download, each
 * step's own button for its own.
 */
import { computed, onBeforeUnmount, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import ConsentDialog from '../components/ConsentDialog.vue'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { useConfigStore } from '../composables/useConfig'
import {
  practiceWord,
  useSetupAssistant,
  type AssistantStep,
} from '../composables/useSetupAssistant'
import { RECOMMENDED_NOTE_TYPE, recommendedNoteTypeAction } from '../domain/fieldMapping'
import { formatShortcut } from '../domain/shortcuts'
import type { MessageKey } from '../i18n'

const props = defineProps<{
  /** Delays for the tests; the defaults are one and two seconds. */
  accessibilityPollMs?: number
  ankiPollMs?: number
}>()

const emit = defineEmits<{
  close: []
  /** Whether the "try" step is shown: it reports capture errors itself. */
  trying: [shown: boolean]
}>()

const STEP_TITLES: Record<AssistantStep, MessageKey> = {
  anki: 'assistant.steps.anki',
  ankiconnect: 'assistant.steps.ankiconnect',
  running: 'assistant.steps.running',
  note_type: 'assistant.steps.note_type',
  accessibility: 'assistant.steps.accessibility',
  try: 'assistant.steps.try',
}

/** The steps whose problems the Anki connection's feedback line is about. */
const ANKI_STEPS: readonly AssistantStep[] = ['anki', 'ankiconnect', 'running']

const { t } = useI18n()
const anki = useAnkiConnectionContext()
const store = useConfigStore()
const assistant = useSetupAssistant({
  anki,
  store,
  accessibilityPollMs: props.accessibilityPollMs,
  ankiPollMs: props.ankiPollMs,
})
const {
  steps,
  current,
  done,
  ready,
  accessibility,
  running,
  consent,
  canSetUpEverything,
  provisionedHere,
  requesting,
  noteTypeError,
  accessibilityError,
  tryOutcome,
  restarting,
} = assistant
const { state, activity, feedback, models, canInstallAnki, ankiVersion, pendingConsent } = anki

const online = computed(() => state.value === 'online')
const busy = computed(() => activity.value !== null || running.value)
const needsPermission = computed(() => accessibility.value?.required === true)
const savedModel = computed(() => store.saved.value.anki.model_name)
const usesRecommended = computed(() => savedModel.value === RECOMMENDED_NOTE_TYPE)
const hotkey = computed(() => formatShortcut(store.saved.value.hotkeys))
const word = computed(() => practiceWord(store.saved.value))
const position = computed(() => steps.value.indexOf(current.value))

watch(
  () => current.value === 'try',
  (shown) => emit('trying', shown),
  { immediate: true },
)
onBeforeUnmount(() => emit('trying', false))

async function finish(finished: boolean): Promise<void> {
  await assistant.close(finished)
  emit('close')
}
</script>

<template>
  <section class="pane-layout assistant" data-test="setup-assistant">
    <header class="assistant-header">
      <div>
        <h3 class="pane-title">{{ t('assistant.title') }}</h3>
        <p class="input-hint">{{ t('assistant.intro') }}</p>
      </div>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="assistant-dismiss"
        @click="finish(false)"
      >
        {{ t('assistant.dismiss') }}
      </button>
    </header>

    <div class="assistant-everything">
      <button
        type="button"
        class="btn btn-primary"
        data-test="set-up-everything"
        :disabled="!canSetUpEverything"
        @click="assistant.setUpEverything()"
      >
        {{ running ? t('assistant.everythingRunning') : t('assistant.everything') }}
      </button>
      <span class="input-hint">{{ t('assistant.everythingHint') }}</span>
    </div>

    <div class="assistant-body">
      <ol class="assistant-steps">
        <li v-for="(step, index) in steps" :key="step">
          <button
            type="button"
            :class="['assistant-step', { current: step === current, done: done[step] }]"
            :aria-current="step === current ? 'step' : undefined"
            :data-test="`step-${step}`"
            :data-done="done[step]"
            @click="assistant.go(step)"
          >
            <span class="assistant-mark" aria-hidden="true">{{
              done[step] ? '✓' : index + 1
            }}</span>
            <span>{{ t(STEP_TITLES[step]) }}</span>
          </button>
        </li>
      </ol>

      <div class="assistant-detail" data-test="assistant-detail" :data-step="current">
        <p v-if="!ready" class="input-hint spinner">{{ t('assistant.checking') }}</p>

        <template v-else>
          <h4 class="assistant-detail-title">
            {{
              t('assistant.stepOf', {
                number: position + 1,
                total: steps.length,
                title: t(STEP_TITLES[current]),
              })
            }}
          </h4>

          <template v-if="current === 'anki'">
            <p v-if="done.anki" class="success-notice-card">{{ t('assistant.anki.done') }}</p>
            <template v-else>
              <p>{{ t('assistant.anki.body') }}</p>
              <div class="assistant-actions">
                <button
                  v-if="canInstallAnki"
                  type="button"
                  class="btn btn-primary"
                  data-test="install-anki"
                  :disabled="busy"
                  @click="anki.requestInstall()"
                >
                  {{
                    activity === 'install'
                      ? t('anki.installing')
                      : t('anki.install', { version: ankiVersion })
                  }}
                </button>
                <button
                  type="button"
                  class="btn btn-secondary"
                  data-test="rescan"
                  :disabled="busy"
                  @click="anki.rescan()"
                >
                  {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
                </button>
              </div>
              <p class="input-hint">
                {{ canInstallAnki ? t('anki.installManual') : t('anki.installUnsupported') }}
              </p>
            </template>
          </template>

          <template v-else-if="current === 'ankiconnect'">
            <p v-if="done.ankiconnect" class="success-notice-card">
              {{ t('assistant.ankiconnect.done') }}
            </p>
            <template v-else>
              <p>{{ t('anki.pluginBody') }}</p>
              <div class="assistant-actions">
                <button
                  type="button"
                  class="btn btn-primary"
                  data-test="provision"
                  :disabled="busy"
                  @click="anki.requestProvision()"
                >
                  {{ activity === 'provision' ? t('anki.provisioning') : t('anki.pluginInstall') }}
                </button>
                <button
                  type="button"
                  class="btn btn-secondary"
                  data-test="rescan"
                  :disabled="busy"
                  @click="anki.rescan()"
                >
                  {{ activity === 'rescan' ? t('anki.rescanning') : t('anki.rescan') }}
                </button>
              </div>
            </template>
          </template>

          <template v-else-if="current === 'running'">
            <p v-if="done.running" class="success-notice-card">
              {{ t('assistant.running.done') }}
            </p>
            <template v-else>
              <p>{{ t('assistant.running.body') }}</p>
              <p
                :class="provisionedHere ? 'notice warning' : 'input-hint'"
                data-test="restart-anki-hint"
              >
                {{
                  provisionedHere
                    ? t('assistant.running.justInstalled')
                    : t('assistant.running.restart')
                }}
              </p>
              <div class="assistant-actions">
                <button
                  type="button"
                  class="btn btn-primary"
                  data-test="launch"
                  :disabled="busy"
                  @click="anki.launch()"
                >
                  {{ activity === 'launch' ? t('anki.waiting') : t('anki.launch') }}
                </button>
              </div>
              <p class="input-hint">{{ t('assistant.running.firstStart') }}</p>
            </template>
          </template>

          <template v-else-if="current === 'note_type'">
            <p v-if="!online" class="notice">{{ t('assistant.noteType.offline') }}</p>
            <template v-else>
              <p v-if="done.note_type" class="success-notice-card" data-test="note-type-done">
                {{ t('assistant.noteType.done', { model: savedModel }) }}
              </p>
              <p v-else>{{ t('assistant.noteType.body', { name: RECOMMENDED_NOTE_TYPE }) }}</p>
              <p v-if="done.note_type && !usesRecommended" class="input-hint">
                {{ t('assistant.noteType.keep', { name: RECOMMENDED_NOTE_TYPE }) }}
              </p>
              <div v-if="!done.note_type || !usesRecommended" class="assistant-actions">
                <button
                  type="button"
                  :class="['btn', done.note_type ? 'btn-secondary' : 'btn-primary']"
                  data-test="set-up-note-type"
                  :disabled="busy"
                  @click="assistant.setUpNoteType()"
                >
                  {{
                    activity === 'note_type'
                      ? t('noteType.creating')
                      : t(recommendedNoteTypeAction(models))
                  }}
                </button>
              </div>
            </template>
            <p
              v-if="noteTypeError"
              class="inline-result error"
              role="alert"
              data-test="note-type-error"
            >
              {{ t(noteTypeError.key, noteTypeError.params ?? {}) }}
            </p>
          </template>

          <template v-else-if="current === 'accessibility'">
            <p v-if="done.accessibility" class="success-notice-card">
              {{ t('assistant.accessibility.done') }}
            </p>
            <template v-else>
              <p>{{ t('assistant.accessibility.body') }}</p>
              <div class="assistant-actions">
                <button
                  type="button"
                  class="btn btn-primary"
                  data-test="open-accessibility"
                  :disabled="requesting"
                  @click="assistant.requestAccessibility()"
                >
                  {{
                    requesting
                      ? t('assistant.accessibility.opening')
                      : t('assistant.accessibility.open')
                  }}
                </button>
              </div>
              <p class="input-hint">{{ t('assistant.accessibility.how') }}</p>
              <p class="input-hint spinner" data-test="accessibility-watching">
                {{ t('assistant.accessibility.watching') }}
              </p>
              <details class="assistant-more">
                <summary>{{ t('assistant.accessibility.staleTitle') }}</summary>
                <p class="input-hint">{{ t('assistant.accessibility.stale') }}</p>
              </details>
              <p
                v-if="accessibilityError"
                class="inline-result error"
                role="alert"
                data-test="accessibility-error"
              >
                {{ t(accessibilityError.key, accessibilityError.params ?? {}) }}
              </p>
            </template>
          </template>

          <template v-else>
            <p>{{ t('assistant.try.body', { hotkey }) }}</p>
            <p class="practice-word" data-test="practice-word">{{ word }}</p>
            <p v-if="!tryOutcome" class="input-hint spinner">{{ t('assistant.try.waiting') }}</p>
            <p
              v-else-if="tryOutcome.kind === 'success'"
              class="success-notice-card"
              role="status"
              data-test="try-success"
            >
              {{ t('assistant.try.success') }}
            </p>
            <template v-else>
              <p class="inline-result error" role="alert" data-test="try-error">
                {{ t('assistant.try.failed', { message: tryOutcome.message }) }}
              </p>
              <template v-if="needsPermission">
                <p v-if="!done.accessibility" class="input-hint">
                  {{ t('assistant.try.notAllowed', { step: t('assistant.steps.accessibility') }) }}
                </p>
                <div v-else class="notice warning assistant-restart" data-test="restart-offer">
                  <span>{{ t('assistant.try.restartHint') }}</span>
                  <button
                    type="button"
                    class="btn btn-secondary btn-mini"
                    data-test="restart-app"
                    :disabled="restarting"
                    @click="assistant.restart()"
                  >
                    {{ restarting ? t('assistant.try.restarting') : t('assistant.try.restart') }}
                  </button>
                </div>
              </template>
            </template>
            <p class="input-hint">{{ t('assistant.try.anyApp', { hotkey }) }}</p>
          </template>

          <p
            v-if="feedback && ANKI_STEPS.includes(current)"
            :class="['inline-result', feedback.tone]"
            data-test="anki-feedback"
          >
            {{ t(feedback.key, feedback.params ?? {}) }}
          </p>

          <footer class="assistant-nav">
            <button
              v-if="current !== 'try'"
              type="button"
              class="btn btn-secondary"
              data-test="assistant-next"
              @click="assistant.skip()"
            >
              {{ done[current] ? t('assistant.next') : t('assistant.skip') }}
            </button>
            <button
              v-else
              type="button"
              class="btn btn-primary"
              data-test="assistant-done"
              @click="finish(true)"
            >
              {{ t('assistant.done') }}
            </button>
          </footer>
        </template>
      </div>
    </div>

    <ConsentDialog
      v-if="consent"
      :actions="consent"
      :anki-version="ankiVersion"
      @agree="assistant.agree()"
      @cancel="assistant.cancel()"
    >
      <template #after>
        <div class="consent-plan" data-test="consent-plan">
          <h4>{{ t('consent.planHeading') }}</h4>
          <ul>
            <li>{{ t('consent.planLaunch') }}</li>
            <li>{{ t('consent.planNoteType', { name: RECOMMENDED_NOTE_TYPE }) }}</li>
            <li v-if="needsPermission">{{ t('consent.planAccessibility') }}</li>
          </ul>
        </div>
      </template>
    </ConsentDialog>
    <ConsentDialog
      v-else-if="pendingConsent"
      :actions="[pendingConsent]"
      :anki-version="ankiVersion"
      @agree="assistant.agreeSingle()"
      @cancel="anki.cancelConsent()"
    />
  </section>
</template>
