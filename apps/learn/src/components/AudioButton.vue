<script setup lang="ts">
/**
 * A "play" button for a word or sentence (see useAudio for where the sound
 * comes from), with its status read out politely: loading, nothing to play,
 * or why playback failed. While a recording loads the button is
 * `aria-disabled`, not disabled, so it keeps the focus of a keyboard user
 * who pressed it (a disabled button drops focus to the page).
 */
import { computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'

import { useAudio, type AudioSource } from '../composables/useAudio'

const props = defineProps<{
  source: AudioSource
  label: string
  /** Try to play once on mount (listening exercises). */
  autoplay?: boolean
}>()

const { t } = useI18n()
const audio = useAudio()
const { status, error } = audio

const message = computed(() => {
  if (status.value === 'loading') return t('exercise.audioLoading')
  if (status.value === 'unavailable') return t('exercise.audioUnavailable')
  if (status.value === 'failed') return t('exercise.audioFailed', { error: error.value ?? '' })
  return ''
})

function play(): void {
  void audio.play(props.source)
}

onMounted(() => {
  if (props.autoplay) void audio.play(props.source, { quiet: true })
})
</script>

<template>
  <div class="audio-control">
    <button
      type="button"
      class="btn btn-secondary audio-button"
      data-test="play-audio"
      :aria-disabled="status === 'loading' || undefined"
      @click="play"
    >
      <span aria-hidden="true">▶</span> {{ label }}
    </button>
    <span
      class="input-hint"
      :class="{ 'audio-error': status === 'failed' }"
      role="status"
      data-test="audio-status"
      >{{ message }}</span
    >
  </div>
</template>
