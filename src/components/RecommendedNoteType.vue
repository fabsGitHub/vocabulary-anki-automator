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
 * user saves. The result line stays until the place switches to another
 * note type.
 */
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import type { RecommendedNoteType } from '../api'
import { useAnkiConnectionContext } from '../composables/useAnkiConnection'
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

const name = RECOMMENDED_NOTE_TYPE
const online = computed(() => state.value === 'online')
const selected = computed(() => props.modelName.trim() === name)
const exists = computed(() => models.value.includes(name))
const offered = computed(() => online.value && !(selected.value && exists.value))
const running = ref(false)
const result = ref<{ tone: 'success' | 'error'; key: MessageKey; params: Record<string, string> }>()

watch(
  () => props.modelName,
  (model) => {
    if (model.trim() !== name) result.value = undefined
  },
)

async function setUp(): Promise<void> {
  running.value = true
  result.value = undefined
  try {
    const outcome = await anki.createRecommendedNoteType()
    if (!outcome) return
    if (!outcome.ok) {
      result.value = { tone: 'error', key: 'noteType.failed', params: { error: outcome.error } }
      return
    }
    const { noteType } = outcome
    emit('use', noteType)
    result.value = {
      tone: 'success',
      key: noteType.created ? 'noteType.created' : 'noteType.reused',
      params: { name: noteType.model_name },
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
      {{ t(result.key, result.params) }}
    </p>
  </div>
</template>
