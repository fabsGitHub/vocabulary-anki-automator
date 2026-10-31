<script setup lang="ts">
/**
 * The word bank of the sentence exercises (sentence builder, listening
 * sentence): tapping a tile in the bank places it at the end of the answer,
 * tapping a placed tile takes it back. Tiles are identified by id, never by
 * text, because two tiles can read the same. A placed tile leaves a gap in
 * the bank so the other tiles do not jump around.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Tile } from '../../platform/types'

const props = defineProps<{
  tiles: Tile[]
  /** Language of the tile texts. */
  lang: string
  disabled: boolean
}>()

/** The placed tile ids, in answer order. */
const placed = defineModel<number[]>({ required: true })

const { t } = useI18n()

const byId = computed(() => new Map(props.tiles.map((tile) => [tile.id, tile])))
const placedTiles = computed(() =>
  placed.value.map((id) => byId.value.get(id)).filter((tile): tile is Tile => tile !== undefined),
)
const used = computed(() => new Set(placed.value))

function place(tile: Tile): void {
  if (props.disabled || used.value.has(tile.id)) return
  placed.value = [...placed.value, tile.id]
}

function remove(tile: Tile): void {
  if (props.disabled) return
  placed.value = placed.value.filter((id) => id !== tile.id)
}
</script>

<template>
  <div class="tile-board">
    <div
      class="tile-answer"
      role="group"
      :aria-label="t('exercise.yourAnswer')"
      data-test="tile-answer"
    >
      <span v-if="placedTiles.length === 0" class="tile-placeholder">{{
        t('exercise.tapToPlace')
      }}</span>
      <button
        v-for="tile in placedTiles"
        :key="tile.id"
        type="button"
        class="tile is-placed"
        :lang="lang"
        :disabled="disabled"
        :data-test="`placed-${tile.id}`"
        @click="remove(tile)"
      >
        {{ tile.text }}
      </button>
    </div>
    <div class="tile-bank" role="group" :aria-label="t('exercise.wordBank')" data-test="tile-bank">
      <button
        v-for="tile in tiles"
        :key="tile.id"
        type="button"
        class="tile"
        :class="{ 'is-used': used.has(tile.id) }"
        :lang="lang"
        :disabled="disabled || used.has(tile.id)"
        :aria-hidden="used.has(tile.id) || undefined"
        :tabindex="used.has(tile.id) ? -1 : undefined"
        :data-test="`tile-${tile.id}`"
        @click="place(tile)"
      >
        {{ tile.text }}
      </button>
    </div>
  </div>
</template>
