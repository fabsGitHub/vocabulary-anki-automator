<script setup lang="ts">
/**
 * Learn's main window (Milestone I; L2's "main-window mount" is this window,
 * docs/architecture.md § 12 item 6).
 *
 * On mount it asks for the daily prompt state and, when today has not been
 * answered, shows the prompt before anything else renders: today's queue
 * depends on the answer. Then five tabs: the review session, practice
 * exercises, lessons, stats, and settings with the Anki export and import.
 * A tab is mounted on its first visit and kept afterwards, so switching tabs
 * never loses a half-done review. The menu reopens the prompt to change
 * today's goal, after which the review rebuilds its queue, and opens the
 * credits and licenses of the bundled data. While the prompt or the credits
 * are open, everything behind them is inert: out of reach of the keyboard,
 * the pointer and assistive technology.
 */
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import AppMenu, { type MenuItem } from './components/AppMenu.vue'
import CreditsDialog from './components/CreditsDialog.vue'
import DailyPromptModal from './components/DailyPromptModal.vue'
import TabPanel from './components/TabPanel.vue'
import { useDailyPrompt } from './composables/useDailyPrompt'
import { useClock, usePlatform } from './context'
import { UI_LANGS, UI_LANG_LABELS, chooseUiLang, type MessageKey } from './i18n'
import AnkiView from './views/AnkiView.vue'
import LessonsView from './views/LessonsView.vue'
import PracticeView from './views/PracticeView.vue'
import ReviewSession from './views/ReviewSession.vue'
import StatsView from './views/StatsView.vue'

type TabId = 'review' | 'practice' | 'lessons' | 'stats' | 'anki'

const TABS: { id: TabId; label: MessageKey }[] = [
  { id: 'review', label: 'tabs.review' },
  { id: 'practice', label: 'tabs.practice' },
  { id: 'lessons', label: 'tabs.lessons' },
  { id: 'stats', label: 'tabs.stats' },
  { id: 'anki', label: 'tabs.anki' },
]

const { t, locale } = useI18n()
const prompt = useDailyPrompt(usePlatform(), useClock())
const {
  phase,
  loadError,
  open,
  started,
  busy,
  actionError,
  reopenError,
  session,
  round,
  suggestedCount,
} = prompt

const activeTab = ref<TabId>('review')
const visited = reactive(new Set<TabId>(['review']))
const creditsOpen = ref(false)
/** A dialog is open: the page behind it is inert. */
const covered = computed(() => open.value || creditsOpen.value)

const menuItems = computed<MenuItem[]>(() => [
  { id: 'change-goal', label: t('menu.changeGoal'), disabled: busy.value || open.value },
  { id: 'credits', label: t('menu.credits') },
])
const newGoal = computed(() => session.value?.requested_new_count ?? null)
const goalText = computed(() => {
  if (newGoal.value === null) return null
  return newGoal.value > 0 ? t('goal.newWords', { count: newGoal.value }) : t('goal.reviewsOnly')
})

function selectTab(id: TabId): void {
  activeTab.value = id
  visited.add(id)
}

/** Arrow keys, Home and End move between tabs (WAI-ARIA tabs pattern). */
function onTabKeydown(event: KeyboardEvent, index: number): void {
  const last = TABS.length - 1
  const target =
    event.key === 'ArrowRight'
      ? (index + 1) % TABS.length
      : event.key === 'ArrowLeft'
        ? (index + last) % TABS.length
        : event.key === 'Home'
          ? 0
          : event.key === 'End'
            ? last
            : null
  if (target === null) return
  event.preventDefault()
  selectTab(TABS[target].id)
  document.getElementById(`tab-${TABS[target].id}`)?.focus()
}

function onMenu(id: string): void {
  if (id === 'change-goal') void prompt.reopen()
  else if (id === 'credits') creditsOpen.value = true
}

function clearReopenError(): void {
  reopenError.value = null
}

function switchUiLang(event: Event): void {
  chooseUiLang((event.target as HTMLSelectElement).value)
}

function onWindowFocus(): void {
  void prompt.checkNewDay()
}

onMounted(() => {
  window.addEventListener('focus', onWindowFocus)
  void prompt.load()
})

onBeforeUnmount(() => window.removeEventListener('focus', onWindowFocus))
</script>

<template>
  <div class="view learn-shell">
    <div class="learn-body" :inert="covered || undefined" data-test="app-body">
      <header class="mac-header">
        <div class="header-titles">
          <h1>{{ t('app.title') }}</h1>
          <p>{{ goalText ?? t('app.subtitle') }}</p>
        </div>
        <div class="controls-cluster">
          <select
            class="mac-select compact-select"
            data-test="ui-lang"
            :value="locale"
            :aria-label="t('app.uiLanguage')"
            :title="t('app.uiLanguage')"
            @change="switchUiLang"
          >
            <option v-for="lang in UI_LANGS" :key="lang" :value="lang" :lang="lang">
              {{ UI_LANG_LABELS[lang] }}
            </option>
          </select>
          <AppMenu v-if="started" :items="menuItems" @select="onMenu" />
        </div>
      </header>

      <div v-if="reopenError" class="notice error toast-row" role="alert" data-test="reopen-error">
        <span>{{ t('app.reopenFailed', { error: reopenError }) }}</span>
        <button
          type="button"
          class="close-btn"
          :aria-label="t('app.dismiss')"
          @click="clearReopenError"
        >
          &times;
        </button>
      </div>

      <template v-if="started">
        <div v-if="phase === 'error'" class="notice error toast-row" role="alert">
          <span>{{ t('app.loadFailed', { error: loadError ?? '' }) }}</span>
          <button type="button" class="btn btn-secondary btn-mini" @click="prompt.load()">
            {{ t('app.retry') }}
          </button>
        </div>

        <nav class="segmented-bar" role="tablist" :aria-label="t('tabs.label')">
          <button
            v-for="(tab, index) in TABS"
            :id="`tab-${tab.id}`"
            :key="tab.id"
            type="button"
            role="tab"
            :aria-selected="activeTab === tab.id"
            :aria-controls="`panel-${tab.id}`"
            :tabindex="activeTab === tab.id ? 0 : -1"
            :class="['segment-btn', { active: activeTab === tab.id }]"
            :data-test="`tab-${tab.id}`"
            @click="selectTab(tab.id)"
            @keydown="onTabKeydown($event, index)"
          >
            {{ t(tab.label) }}
          </button>
        </nav>

        <main class="pane-scroll">
          <template v-for="tab in TABS" :key="tab.id">
            <TabPanel
              v-if="visited.has(tab.id)"
              :id="`panel-${tab.id}`"
              :labelled-by="`tab-${tab.id}`"
              :selected="activeTab === tab.id"
              :interactive="!covered"
            >
              <ReviewSession
                v-if="tab.id === 'review'"
                :round="round"
                :new-goal="newGoal"
                @change-goal="prompt.reopen()"
              />
              <PracticeView v-else-if="tab.id === 'practice'" />
              <LessonsView v-else-if="tab.id === 'lessons'" />
              <StatsView v-else-if="tab.id === 'stats'" />
              <AnkiView v-else />
            </TabPanel>
          </template>
        </main>
      </template>

      <p v-else-if="phase === 'loading'" class="input-hint spinner" data-test="app-loading">
        {{ t('app.loading') }}
      </p>
      <div v-else-if="phase === 'error'" class="onboarding-notice warning" data-test="app-error">
        <p role="alert">{{ t('app.loadFailed', { error: loadError ?? '' }) }}</p>
        <button type="button" class="btn btn-primary" data-test="app-retry" @click="prompt.load()">
          {{ t('app.retry') }}
        </button>
      </div>
    </div>

    <DailyPromptModal
      v-if="open"
      :initial-count="suggestedCount"
      :busy="busy"
      :error="actionError"
      @start="prompt.submit($event)"
      @dismiss="prompt.dismiss()"
    />
    <CreditsDialog v-if="creditsOpen" @close="creditsOpen = false" />
  </div>
</template>
