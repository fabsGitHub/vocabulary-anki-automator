/**
 * An in-memory stand-in for the Tauri backend, shared by the frontend tests.
 *
 * Test files replace `@tauri-apps/api/core`, `/event` and `/window` with the
 * modules exported here (`vi.mock(..., async () => (await import(...)).coreModule)`),
 * so the code under test runs its real `src/api` wrappers and every assertion
 * sees the exact command names and argument objects that would cross the IPC
 * boundary. Commands without a handler reject the way Tauri rejects an
 * unknown command, which makes an unexpected call fail loudly.
 */

import { vi } from 'vitest'

type CommandHandler = (args: Record<string, unknown> | undefined) => unknown

export interface InvokeCall {
  cmd: string
  args: Record<string, unknown> | undefined
}

interface TauriEvent {
  event: string
  id: number
  payload: unknown
}

const handlers = new Map<string, CommandHandler>()
const listeners = new Map<string, Set<(event: TauriEvent) => void>>()
let nextEventId = 1

/** Every `invoke`, in call order. */
export const invokeCalls: InvokeCall[] = []

export const windowState = { label: 'main' }

/** `getCurrentWindow().hide`. */
export const hideWindow = vi.fn(async () => {})

/** Answers `cmd` with `handler(args)`, or with `handler` itself when it is not a function. */
export function onCommand(cmd: string, handler: unknown): void {
  handlers.set(
    cmd,
    typeof handler === 'function' ? (handler as CommandHandler) : () => structuredCopy(handler),
  )
}

/** The commands invoked so far, in order. */
export function commandNames(): string[] {
  return invokeCalls.map((call) => call.cmd)
}

export function callsTo(cmd: string): InvokeCall[] {
  return invokeCalls.filter((call) => call.cmd === cmd)
}

/** Delivers `payload` to every listener of `event`, as the backend's `emit` would. */
export function emitEvent(event: string, payload: unknown = null): void {
  for (const listener of [...(listeners.get(event) ?? [])]) {
    listener({ event, id: nextEventId++, payload: structuredCopy(payload) })
  }
}

export function listenerCount(event: string): number {
  return listeners.get(event)?.size ?? 0
}

export function resetTauri(): void {
  handlers.clear()
  listeners.clear()
  invokeCalls.length = 0
  windowState.label = 'main'
  hideWindow.mockReset()
  hideWindow.mockResolvedValue(undefined)
  coreModule.invoke.mockClear()
  eventModule.listen.mockClear()
}

/** A promise with its resolve/reject exposed, to control when a fake command answers. */
export function deferred<T>(): {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason: unknown) => void
} {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/** Like the JSON round trip of real IPC: callers never share objects with the fake backend. */
function structuredCopy<T>(value: T): T {
  return value === undefined ? value : (JSON.parse(JSON.stringify(value)) as T)
}

export const coreModule = {
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    invokeCalls.push({ cmd, args: args === undefined ? undefined : structuredCopy(args) })
    const handler = handlers.get(cmd)
    if (!handler) throw `command ${cmd} not found`
    return handler(args)
  }),
}

export const eventModule = {
  listen: vi.fn(async (event: string, handler: (event: TauriEvent) => void) => {
    const set = listeners.get(event) ?? new Set()
    set.add(handler)
    listeners.set(event, set)
    return () => {
      set.delete(handler)
    }
  }),
}

export const windowModule = {
  getCurrentWindow: () => ({ label: windowState.label, hide: hideWindow }),
}
