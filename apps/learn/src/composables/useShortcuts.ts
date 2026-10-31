/**
 * Single-key shortcuts (Space, 1–4) for the view that is on screen.
 *
 * Every tab stays mounted once visited, so that a half-done review survives
 * a look at the stats; several views therefore listen on `window` at once.
 * Each tab panel provides whether it is the one in front and nothing covers
 * it (the daily prompt), and a shortcut only fires there. It never fires
 * while typing in a field, with a modifier held (Cmd+1 stays the system's),
 * or for Space/Enter on a focused button, which the button itself handles.
 */

import {
  computed,
  inject,
  onBeforeUnmount,
  onMounted,
  provide,
  type ComputedRef,
  type InjectionKey,
} from 'vue'

const PANEL_ACTIVE_KEY: InjectionKey<ComputedRef<boolean>> = Symbol('panel-active')

/** Called by a tab panel for everything inside it. */
export function providePanelActive(active: ComputedRef<boolean>): void {
  provide(PANEL_ACTIVE_KEY, active)
}

/** Whether this component's tab panel is in front; always true outside one. */
export function usePanelActive(): ComputedRef<boolean> {
  return inject(
    PANEL_ACTIVE_KEY,
    computed(() => true),
    false,
  )
}

/** A key the handler took; the event's default (scrolling, for Space) is prevented. */
export type Handled = boolean

/** Listens for single-key shortcuts while the panel is active. */
export function useShortcuts(handler: (key: string, event: KeyboardEvent) => Handled): void {
  const active = usePanelActive()

  function onKeydown(event: KeyboardEvent): void {
    if (!active.value || event.defaultPrevented || event.repeat || event.isComposing) return
    if (event.ctrlKey || event.metaKey || event.altKey) return
    if (isTextEntry(event.target)) return
    if ((event.key === ' ' || event.key === 'Enter') && isActivatable(event.target)) return
    if (handler(event.key, event)) event.preventDefault()
  }

  onMounted(() => window.addEventListener('keydown', onKeydown))
  onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))
}

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
}

function isActivatable(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest('button, a[href], summary, [role="button"], [role="tab"], [role="menuitem"]') !==
      null
  )
}
