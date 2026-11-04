<script setup lang="ts">
/**
 * "Create the recommended note type" for one place that picks a note type:
 * the Anki tab and each "My languages" entry (`create_recommended_note_type`,
 * docs/architecture.md § 8.2). It shows while Anki is online, unless the
 * place uses the recommended note type and Anki has it; when Anki already has
 * it, the button only selects it, since the backend reuses it and never
 * changes it.
 *
 * The click runs through the shared connection, so it never overlaps another
 * Anki operation and the note type lists are read again afterwards. On
 * success it emits `use` with what the backend set up: the parent selects
 * the note type and its mapping in the form, and nothing is saved until the
 * user saves. The result line says whether that is still to do: only when
 * the click changed the form and the form differs from the saved settings
 * does it ask the user to save. It stays until the place switches to another
 * note type, and a success line until the saved settings change.
 */
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import type { RecommendedNoteType } from '../api'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
import { toSettingsPayload, useConfigStore } from '../composables/useConfig'
import { RECOMMENDED_NOTE_TYPE } from '../domain/fieldMapping'
import type { MessageKey } from '../i18n'

const props = defineProps<{
  /** The note type the place uses now. */
  modelName: string
}>()

const emit = defineEmits<{ use: [noteType: RecommendedNoteType] }>()

const { t } = useI18n()
const anki = useAnkiConnectionContext()
const { state, activity, models } = anki
const store = useConfigStore()

const name = RECOMMENDED_NOTE_TYPE
const online = computed(() => state.value === 'online')
const selected = computed(() => props.modelName.trim() === name)
const exists = computed(() => models.value.includes(name))
const offered = computed(() => online.value && !(selected.value && exists.value))
const running = ref(false)
type Line = { key: MessageKey; params?: Record<string, string> }
const result = ref<{ tone: 'success' | 'error'; lines: Line[] }>()

watch(
  () => props.modelName,
  (model) => {
    if (model.trim() !== name) result.value = undefined
  },
)
// A save (or any other change to the saved settings) outdates what the line
// says about saving.
watch(store.saved, () => {
  if (result.value?.tone === 'success') result.value = undefined
})

/** The form as a save would send it. */
const formState = () => JSON.stringify(toSettingsPayload(store.config.value))

/** What is left to do after the parent selected the note type. */
function followUp(changed: boolean): MessageKey {
  if (!store.dirty.value) return 'noteType.inUse'
  return changed ? 'noteType.saveToUse' : 'noteType.alreadySelected'
}

async function setUp(): Promise<void> {
  running.value = true
  result.value = undefined
  try {
    const outcome = await anki.createRecommendedNoteType()
    if (!outcome) return
    if (!outcome.ok) {
      result.value = {
        tone: 'error',
        lines: [{ key: 'noteType.failed', params: { error: outcome.error } }],
      }
      return
    }
    const { noteType } = outcome
    const before = formState()
    // The parent's handler runs synchronously, so the form is updated below.
    emit('use', noteType)
    result.value = {
      tone: 'success',
      lines: [
        {
          key: noteType.created ? 'noteType.created' : 'noteType.reused',
          params: { name: noteType.model_name },
        },
        { key: followUp(formState() !== before) },
      ],
    }
  } finally {
    running.value = false
  }
}
</script>

<template>
  <div v-if="offered || result" class="note-type-offer" data-test="recommended-note-type">
    <template v-if="offered">
      <button
        type="button"
        class="btn btn-secondary btn-mini"
        data-test="create-note-type"
        :disabled="activity !== null"
        @click="setUp"
      >
        {{ running ? t('noteType.creating') : exists ? t('noteType.use') : t('noteType.create') }}
      </button>
      <span class="input-hint" data-test="note-type-hint">
        {{ exists ? t('noteType.useHint', { name }) : t('noteType.createHint', { name }) }}
      </span>
    </template>
    <p
      v-if="result"
      :class="['inline-result', result.tone]"
      :role="result.tone === 'error' ? 'alert' : 'status'"
      data-test="note-type-result"
    >
      {{ result.lines.map((line) => t(line.key, line.params ?? {})).join(' ') }}
    </p>
  </div>
</template>
