<script setup lang="ts">
/** A labelled progress bar (review session, lessons): `value` of `max` done. */
import { computed } from 'vue'

const props = defineProps<{ value: number; max: number; label: string; text: string }>()

const percent = computed(() =>
  props.max > 0 ? Math.min(100, Math.max(0, (props.value / props.max) * 100)) : 0,
)
</script>

<template>
  <div class="progress-row">
    <div
      class="progress-track"
      role="progressbar"
      :aria-label="label"
      aria-valuemin="0"
      :aria-valuemax="max"
      :aria-valuenow="value"
      :aria-valuetext="text"
      data-test="progress"
    >
      <div class="progress-fill" :style="{ width: `${percent}%` }" />
    </div>
    <span class="progress-text" aria-hidden="true">{{ text }}</span>
  </div>
</template>
