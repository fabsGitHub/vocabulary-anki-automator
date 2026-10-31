<script setup lang="ts">
/**
 * Edits `hotkey_profiles` (Milestone H): extra shortcuts that capture into
 * their own language pair and, optionally, their own deck, without touching
 * the default hotkey's settings.
 */
import { useI18n } from 'vue-i18n'

import type { HotkeyProfile } from '../api'
import LanguageSelect from './LanguageSelect.vue'
import { useConfigStore } from '../composables/useConfig'
import { useIssueText } from '../composables/useIssueText'
import { isSupportedLang } from '../domain/language'
import { LANGUAGE_NAME_KEYS } from '../i18n'

defineProps<{
  /** Deck names Anki reported, offered as suggestions. */
  decks: string[]
}>()

const { t } = useI18n()
const store = useConfigStore()
const config = store.config
const issueText = useIssueText()

function languageName(code: string): string {
  return isSupportedLang(code) ? t(LANGUAGE_NAME_KEYS[code]) : code
}

function uniqueId(base: string): string {
  const taken = new Set(config.value.hotkey_profiles.map((profile) => profile.id))
  let id = base
  for (let n = 2; taken.has(id); n++) id = `${base}-${n}`
  return id
}

function addProfile(): void {
  const { source_lang: source, target_lang: target } = config.value.translation
  config.value.hotkey_profiles.push({
    id: uniqueId(`${source}-${target}`.toLowerCase()),
    label: t('profiles.defaultLabel', {
      source: languageName(source),
      target: languageName(target),
    }),
    shortcut: '',
    source_lang: source,
    target_lang: target,
    deck_name: null,
  })
}

function removeProfile(index: number): void {
  config.value.hotkey_profiles.splice(index, 1)
}

function setDeck(profile: HotkeyProfile, value: string): void {
  profile.deck_name = value.trim() === '' ? null : value
}

function issue(index: number, field: keyof HotkeyProfile): string | null {
  return issueText(`hotkey_profiles.${index}.${field}`)
}
</script>

<template>
  <section class="profiles-editor">
    <div class="pane-title-row">
      <h3 class="pane-title">{{ t('profiles.title') }}</h3>
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="add-profile"
        @click="addProfile"
      >
        {{ t('profiles.add') }}
      </button>
    </div>
    <p class="input-hint">{{ t('profiles.intro') }}</p>

    <p v-if="config.hotkey_profiles.length === 0" class="empty-state" data-test="no-profiles">
      {{ t('profiles.empty') }}
    </p>

    <datalist id="profile-decks">
      <option v-for="deck in decks" :key="deck" :value="deck" />
    </datalist>

    <article
      v-for="(profile, index) in config.hotkey_profiles"
      :key="index"
      class="profile-card"
      data-test="profile"
    >
      <div class="grid-row">
        <div class="field-group">
          <label :for="`profile-${index}-label`">{{ t('profiles.label') }}</label>
          <input
            :id="`profile-${index}-label`"
            v-model="profile.label"
            type="text"
            class="mac-input"
            data-test="profile-label"
            :class="{ 'has-error': issue(index, 'label') }"
            :placeholder="t('profiles.labelPlaceholder')"
          />
          <span v-if="issue(index, 'label')" class="field-error">{{ issue(index, 'label') }}</span>
        </div>
        <div class="field-group">
          <label :for="`profile-${index}-id`">{{ t('profiles.id') }}</label>
          <input
            :id="`profile-${index}-id`"
            v-model="profile.id"
            type="text"
            class="mac-input"
            data-test="profile-id"
            :class="{ 'has-error': issue(index, 'id') }"
          />
          <span v-if="issue(index, 'id')" class="field-error">{{ issue(index, 'id') }}</span>
          <span v-else class="input-hint">{{ t('profiles.idHint') }}</span>
        </div>
      </div>

      <div class="field-group">
        <label :for="`profile-${index}-shortcut`">{{ t('profiles.shortcut') }}</label>
        <input
          :id="`profile-${index}-shortcut`"
          v-model="profile.shortcut"
          type="text"
          class="mac-input"
          data-test="profile-shortcut"
          spellcheck="false"
          autocomplete="off"
          :class="{ 'has-error': issue(index, 'shortcut') }"
          :placeholder="t('capture.hotkeyPlaceholder')"
        />
        <span
          v-if="issue(index, 'shortcut')"
          class="field-error"
          data-test="profile-shortcut-error"
        >
          {{ issue(index, 'shortcut') }}
        </span>
      </div>

      <div class="grid-row">
        <div class="field-group">
          <label :for="`profile-${index}-source`">{{ t('profiles.source') }}</label>
          <LanguageSelect
            :id="`profile-${index}-source`"
            v-model="profile.source_lang"
            :invalid="!!issue(index, 'source_lang')"
          />
          <span v-if="issue(index, 'source_lang')" class="field-error">
            {{ issue(index, 'source_lang') }}
          </span>
        </div>
        <div class="field-group">
          <label :for="`profile-${index}-target`">{{ t('profiles.target') }}</label>
          <LanguageSelect
            :id="`profile-${index}-target`"
            v-model="profile.target_lang"
            :invalid="!!issue(index, 'target_lang')"
          />
          <span v-if="issue(index, 'target_lang')" class="field-error">
            {{ issue(index, 'target_lang') }}
          </span>
        </div>
      </div>

      <div class="profile-footer">
        <div class="field-group profile-deck">
          <label :for="`profile-${index}-deck`">{{ t('profiles.deck') }}</label>
          <input
            :id="`profile-${index}-deck`"
            type="text"
            class="mac-input"
            list="profile-decks"
            data-test="profile-deck"
            :value="profile.deck_name ?? ''"
            :placeholder="t('profiles.deckPlaceholder', { deck: config.anki.deck_name })"
            @input="setDeck(profile, ($event.target as HTMLInputElement).value)"
          />
        </div>
        <button
          type="button"
          class="btn btn-secondary btn-mini"
          data-test="remove-profile"
          @click="removeProfile(index)"
        >
          {{ t('profiles.remove') }}
        </button>
      </div>
    </article>
  </section>
</template>
