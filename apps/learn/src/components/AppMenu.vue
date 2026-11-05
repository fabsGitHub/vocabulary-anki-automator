<script setup lang="ts">
/**
 * The main window's menu. It holds L2's "Change today's new-word goal" action
 * and follows the WAI-ARIA menu button pattern: the arrow keys move between
 * items, Esc and a click elsewhere close it, and focus returns to the button.
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

export interface MenuItem {
  id: string
  label: string
  disabled?: boolean
}

defineProps<{ items: MenuItem[] }>()
const emit = defineEmits<{ select: [id: string] }>()

const { t } = useI18n()
const open = ref(false)
const root = ref<HTMLElement | null>(null)
const button = ref<HTMLButtonElement | null>(null)
const menu = ref<HTMLElement | null>(null)

function menuItems(): HTMLButtonElement[] {
  return [...(menu.value?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])].filter(
    (item) => !item.disabled,
  )
}

async function show(): Promise<void> {
  open.value = true
  await nextTick()
  menuItems()[0]?.focus()
}

function hide(returnFocus: boolean): void {
  if (!open.value) return
  open.value = false
  if (returnFocus) button.value?.focus()
}

function toggle(): void {
  if (open.value) hide(true)
  else void show()
}

function choose(item: MenuItem): void {
  if (item.disabled) return
  hide(true)
  emit('select', item.id)
}

function onMenuKeydown(event: KeyboardEvent): void {
  const items = menuItems()
  const at = items.indexOf(document.activeElement as HTMLButtonElement)
  if (event.key === 'Escape' || event.key === 'Tab') {
    if (event.key === 'Escape') event.preventDefault()
    hide(event.key === 'Escape')
  } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    const step = event.key === 'ArrowDown' ? 1 : -1
    items[(at + step + items.length) % items.length]?.focus()
  } else if (event.key === 'Home' || event.key === 'End') {
    event.preventDefault()
    items[event.key === 'Home' ? 0 : items.length - 1]?.focus()
  }
}

function onDocumentPointer(event: Event): void {
  if (open.value && root.value && !root.value.contains(event.target as Node)) hide(false)
}

onMounted(() => document.addEventListener('pointerdown', onDocumentPointer))
onBeforeUnmount(() => document.removeEventListener('pointerdown', onDocumentPointer))
</script>

<template>
  <div ref="root" class="menu-root">
    <button
      id="app-menu-button"
      ref="button"
      type="button"
      class="btn btn-secondary btn-mini"
      data-test="menu"
      aria-haspopup="menu"
      :aria-expanded="open"
      aria-controls="app-menu"
      @click="toggle"
      @keydown.down.prevent="show"
    >
      {{ t('menu.open') }} <span aria-hidden="true">▾</span>
    </button>
    <ul
      v-show="open"
      id="app-menu"
      ref="menu"
      class="menu-list"
      role="menu"
      aria-labelledby="app-menu-button"
      @keydown="onMenuKeydown"
    >
      <li v-for="item in items" :key="item.id" role="none">
        <button
          type="button"
          role="menuitem"
          class="menu-item"
          tabindex="-1"
          :disabled="item.disabled"
          :data-test="`menu-${item.id}`"
          @click="choose(item)"
        >
          {{ item.label }}
        </button>
      </li>
    </ul>
  </div>
</template>
