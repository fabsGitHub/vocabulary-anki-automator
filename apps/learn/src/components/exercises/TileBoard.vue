<script setup lang="ts">
/**
 * The word bank of the sentence exercises (sentence builder, listening
 * sentence): tapping a tile in the bank places it at the end of the answer,
 * tapping a placed tile takes it back. Tiles are identified by id, never by
 * text, because two tiles can read the same. A placed tile leaves a gap in
 * the bank so the other tiles do not jump around.
 *
 * Keyboard focus stays on the board. The tile a keyboard user activates
 * leaves its row (the bank's copy is disabled, the placed copy removed), and
 * the browser would drop focus to the page, sending them back to the top of
 * the window after every word. Placing a tile therefore moves focus on to
 * the next tile still in the bank; taking one back moves it to the placed
 * tile that took its position, or to the returned tile once the answer is
 * empty. A pointer tap that did not focus the tile moves no focus.
 */
import { computed, nextTick, ref } from 'vue'
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
const answerRow = ref<HTMLElement | null>(null)
const bankRow = ref<HTMLElement | null>(null)

const byId = computed(() => new Map(props.tiles.map((tile) => [tile.id, tile])))
const placedTiles = computed(() =>
  placed.value.map((id) => byId.value.get(id)).filter((tile): tile is Tile => tile !== undefined),
)
const used = computed(() => new Set(placed.value))

/** Whether keyboard focus is on one of the board's tiles. */
function focusOnBoard(): boolean {
  const focused = document.activeElement
  if (!focused) return false
  return Boolean(answerRow.value?.contains(focused) || bankRow.value?.contains(focused))
}

/** The `index`th tile button of `row` (the bank lists every tile, the answer the placed ones). */
function tileButton(row: HTMLElement | null, index: number): HTMLButtonElement | undefined {
  return row?.querySelectorAll<HTMLButtonElement>('button')[index]
}

function bankIndex(tile: Tile): number {
  return props.tiles.findIndex((other) => other.id === tile.id)
}

function focusAfterPlacing(tile: Tile): void {
  const at = bankIndex(tile)
  const after = [...props.tiles.slice(at + 1), ...props.tiles.slice(0, at)]
  const next = after.find((other) => !used.value.has(other.id))
  if (next) tileButton(bankRow.value, bankIndex(next))?.focus()
  else tileButton(answerRow.value, placedTiles.value.length - 1)?.focus()
}

function focusAfterRemoving(tile: Tile, position: number): void {
  const count = placedTiles.value.length
  if (count > 0) tileButton(answerRow.value, Math.min(position, count - 1))?.focus()
  else tileButton(bankRow.value, bankIndex(tile))?.focus()
}

function place(tile: Tile): void {
  if (props.disabled || used.value.has(tile.id)) return
  const keepFocus = focusOnBoard()
  placed.value = [...placed.value, tile.id]
  if (keepFocus) void nextTick(() => focusAfterPlacing(tile))
}

function remove(tile: Tile): void {
  if (props.disabled) return
  const keepFocus = focusOnBoard()
  const position = placedTiles.value.findIndex((other) => other.id === tile.id)
  placed.value = placed.value.filter((id) => id !== tile.id)
  if (keepFocus) void nextTick(() => focusAfterRemoving(tile, position))
}
</script>

<template>
  <div class="tile-board">
    <div
      ref="answerRow"
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
    <div
      ref="bankRow"
      class="tile-bank"
      role="group"
      :aria-label="t('exercise.wordBank')"
      data-test="tile-bank"
    >
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
