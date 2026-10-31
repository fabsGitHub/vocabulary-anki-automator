<script setup lang="ts">
/**
 * One tab's panel. The main window mounts it on the tab's first visit and
 * then only hides it, so a half-done review or lesson survives switching
 * tabs; what it provides tells the views inside whether they are the ones in
 * front (keyboard shortcuts, refresh on show).
 */
import { computed } from 'vue'

import { providePanelActive } from '../composables/useShortcuts'

const props = defineProps<{
  id: string
  labelledBy: string
  selected: boolean
  /** False while something covers the window (the daily prompt). */
  interactive: boolean
}>()

providePanelActive(computed(() => props.selected && props.interactive))
</script>

<template>
  <section
    v-show="selected"
    :id="id"
    class="tab-panel"
    role="tabpanel"
    :aria-labelledby="labelledBy"
  >
    <slot />
  </section>
</template>
