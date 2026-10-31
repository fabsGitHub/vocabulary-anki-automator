/**
 * The Automator's TypeScript side of the IPC contract (docs/architecture.md
 * § 8.2, § 11), checked against what the Rust side really does.
 *
 * `__fixtures__/ipc_shapes.json` is written by `src-tauri/tests/ipc_shapes.rs`
 * from the Rust types and the real invoke handler. Here every sample must
 * pass a schema whose type is compile-time equal to its interface in
 * `./types.ts` (exact keys, literal values, nullability); every command and
 * argument name the backend reads must be what the `./commands.ts` wrapper
 * sends, and its result type what the wrapper promises; every event must be
 * one `./events.ts` subscribes to with the payload type it delivers; and the
 * settings form's parsing and the approval window's messages must accept
 * every value the backend can send.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import architecture from '../../docs/architecture.md?raw'
import { defaultConfig, normalizeConfig, toSettingsPayload } from '../composables/useConfig'
import { CARD_FIELDS, appleStyleMapping, chineseMapping } from '../domain/fieldMapping'
import { formatWarning } from '../domain/warnings'
import { i18n } from '../i18n'
import { emitEvent, invokeCalls, onCommand, resetTauri } from '../test/fakeTauri'
import {
  array,
  base64,
  boolean,
  documentedCommands,
  instant,
  integer,
  literal,
  nullable,
  object,
  string,
  union,
  type Equals,
  type Mismatches,
} from '../test/shape'
import fixture from './__fixtures__/ipc_shapes.json'
import * as commands from './commands'
import { onCaptureError, onCapturesChanged, onTranslationPayload } from './events'
import type * as T from './types'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('../test/fakeTauri')).eventModule)

// ---------------------------------------------------------------------------
// One schema per type of ./types.ts

const scriptMetadata = union(
  object({ kind: literal('none') }),
  object({
    kind: literal('chinese'),
    pinyin: string,
    hsk_level: nullable(integer),
    script_variant: literal('simplified', 'traditional'),
  }),
)
const card = object({
  front: string,
  back: string,
  example_source: string,
  example_target: string,
  notes: string,
  tags: array(string),
  source_lang: string,
  target_lang: string,
  captured_at: instant,
  audio_front: nullable(base64),
  script_metadata: scriptMetadata,
})
const providerError = union(
  object({
    kind: literal('network', 'quota_exceeded', 'malformed'),
    provider: string,
    message: string,
  }),
  object({ kind: literal('http'), provider: string, status: integer, message: string }),
  object({ kind: literal('not_found'), provider: string }),
  object({ kind: literal('unsupported'), provider: string, what: string }),
)
const pipelineStage = literal(
  'translate',
  'rarity',
  'example',
  'example_translate',
  'tts',
  'annotate',
)
const pipelineWarning = object({ stage: pipelineStage, error: providerError })
const pendingCapture = object({
  card,
  warnings: array(pipelineWarning),
  profile_id: nullable(string),
})
const captureOrigin = literal('capture', 'curriculum', 'anki_import')
const storedCapture = object({
  id: integer,
  card,
  origin: captureOrigin,
  synced_to_anki: boolean,
})
const submitResult = object({
  capture_id: integer,
  synced_to_anki: boolean,
  note_ids: array(integer),
  anki_error: nullable(string),
})
const syncFailure = object({ id: integer, error: string })
const syncReport = object({ attempted: integer, synced: integer, failed: array(syncFailure) })
const ankiStatus = object({
  online: boolean,
  version: nullable(integer),
  decks: array(string),
  models: array(string),
  error: nullable(string),
})
const consentState = object({
  anki_install_granted_at: nullable(instant),
  ankiconnect_provision_granted_at: nullable(instant),
})
const consentAction = literal('anki_install', 'ankiconnect_provision')
const setupStatus = object({
  anki_installed: boolean,
  ankiconnect_installed: boolean,
  platform_supported: boolean,
  anki_version: string,
  consent: consentState,
})
const queueItem = union(
  object({ kind: literal('review'), card_id: integer, due_date: instant }),
  object({ kind: literal('new'), hanzi: string, curriculum_order: integer }),
  object({ kind: literal('new_capture'), card_id: integer }),
)
const cardField = literal(
  'front',
  'back',
  'example_source',
  'example_target',
  'example',
  'notes',
  'pinyin',
  'hsk_level',
)
const fieldBinding = object({ anki_field: string, source: cardField })
const fieldMapping = object({ fields: array(fieldBinding), audio_field: nullable(string) })
const hotkeyProfile = object({
  id: string,
  label: string,
  shortcut: string,
  source_lang: string,
  target_lang: string,
  deck_name: nullable(string),
})
const translationConfig = object({
  source_lang: string,
  target_lang: string,
  max_chars: integer,
  provider: string,
  contact_email: nullable(string),
})
const ankiConfig = object({
  connect_url: string,
  deck_name: string,
  model_name: string,
  create_reverse: boolean,
  field_mapping: fieldMapping,
})
const appConfig = object({
  ui_lang: string,
  hotkeys: string,
  hotkey_profiles: array(hotkeyProfile),
  translation: translationConfig,
  anki: ankiConfig,
  consent: consentState,
})
const captureErrorPayload = object({ message: string })

const SCHEMAS = {
  ScriptMetadata: scriptMetadata,
  Card: card,
  ProviderError: providerError,
  PipelineStage: pipelineStage,
  PipelineWarning: pipelineWarning,
  PendingCapture: pendingCapture,
  CaptureOrigin: captureOrigin,
  StoredCapture: storedCapture,
  SubmitResult: submitResult,
  SyncFailure: syncFailure,
  SyncReport: syncReport,
  AnkiStatus: ankiStatus,
  ConsentState: consentState,
  ConsentAction: consentAction,
  SetupStatus: setupStatus,
  QueueItem: queueItem,
  CardField: cardField,
  FieldBinding: fieldBinding,
  FieldMapping: fieldMapping,
  HotkeyProfile: hotkeyProfile,
  TranslationConfig: translationConfig,
  AnkiConfig: ankiConfig,
  AppConfig: appConfig,
  CaptureErrorPayload: captureErrorPayload,
}

/** The interfaces of ./types.ts the schemas stand for, by name. */
interface Types {
  ScriptMetadata: T.ScriptMetadata
  Card: T.Card
  ProviderError: T.ProviderError
  PipelineStage: T.PipelineStage
  PipelineWarning: T.PipelineWarning
  PendingCapture: T.PendingCapture
  CaptureOrigin: T.CaptureOrigin
  StoredCapture: T.StoredCapture
  SubmitResult: T.SubmitResult
  SyncFailure: T.SyncFailure
  SyncReport: T.SyncReport
  AnkiStatus: T.AnkiStatus
  ConsentState: T.ConsentState
  ConsentAction: T.ConsentAction
  SetupStatus: T.SetupStatus
  QueueItem: T.QueueItem
  CardField: T.CardField
  FieldBinding: T.FieldBinding
  FieldMapping: T.FieldMapping
  HotkeyProfile: T.HotkeyProfile
  TranslationConfig: T.TranslationConfig
  AnkiConfig: T.AnkiConfig
  AppConfig: T.AppConfig
  CaptureErrorPayload: T.CaptureErrorPayload
}

// A compile error here names the type whose schema differs from its interface.
const schemasMatchTypes: [Mismatches<typeof SCHEMAS, Types>] extends [never]
  ? true
  : Mismatches<typeof SCHEMAS, Types> = true

type ShapeName = keyof typeof SCHEMAS
const samples = fixture.shapes as Record<ShapeName, unknown[]>
const sample = <K extends ShapeName>(name: K, index = 0): Types[K] =>
  SCHEMAS[name].check(samples[name][index]) as Types[K]

// ---------------------------------------------------------------------------
// The commands of ./commands.ts and their result types

type Wrapper = Exclude<keyof typeof commands, 'errorMessage'>

/** The result types of the § 8.2 table, by the name the fixture spells them with. */
interface Results {
  AppConfig: T.AppConfig
  null: null
  'PendingCapture | null': T.PendingCapture | null
  SubmitResult: T.SubmitResult
  AnkiStatus: T.AnkiStatus
  'string[]': string[]
  string: string
  SetupStatus: T.SetupStatus
  ConsentState: T.ConsentState
  boolean: boolean
  'StoredCapture[]': T.StoredCapture[]
  SyncReport: T.SyncReport
  never: never
}

const RETURNS = {
  getCurrentSettings: 'AppConfig',
  applySettings: 'null',
  getPendingPayload: 'PendingCapture | null',
  discardPendingPayload: 'null',
  executeCardGeneration: 'SubmitResult',
  captureText: 'null',
  ankiStatus: 'AnkiStatus',
  ankiModelFieldNames: 'string[]',
  launchAnkiDesktop: 'string',
  getSetupStatus: 'SetupStatus',
  grantConsent: 'ConsentState',
  downloadAndInstallAnki: 'string',
  autoProvisionAnkiconnect: 'string',
  checkAnkiconnectInstalled: 'boolean',
  listRecentCaptures: 'StoredCapture[]',
  deleteCapture: 'boolean',
  syncPendingCaptures: 'SyncReport',
  captureInProgress: 'boolean',
  restartApp: 'never',
} as const satisfies Record<Wrapper, keyof Results>

type WrongResults = {
  [F in Wrapper]: Equals<
    Results[(typeof RETURNS)[F]],
    Awaited<ReturnType<(typeof commands)[F]>>
  > extends true
    ? never
    : F
}[Wrapper]
// A compile error here names the wrapper whose result type is not the backend's.
const resultsMatch: [WrongResults] extends [never] ? true : WrongResults = true

/** Calls each wrapper with arguments of its parameter types. */
const CALLS: Record<Wrapper, () => Promise<unknown>> = {
  getCurrentSettings: () => commands.getCurrentSettings(),
  applySettings: () => commands.applySettings(sample('AppConfig')),
  getPendingPayload: () => commands.getPendingPayload(),
  discardPendingPayload: () => commands.discardPendingPayload(),
  executeCardGeneration: () => commands.executeCardGeneration(sample('Card')),
  captureText: () => commands.captureText('house', null),
  ankiStatus: () => commands.ankiStatus(),
  ankiModelFieldNames: () => commands.ankiModelFieldNames('Basic'),
  launchAnkiDesktop: () => commands.launchAnkiDesktop(),
  getSetupStatus: () => commands.getSetupStatus(),
  grantConsent: () => commands.grantConsent('anki_install'),
  downloadAndInstallAnki: () => commands.downloadAndInstallAnki(),
  autoProvisionAnkiconnect: () => commands.autoProvisionAnkiconnect(),
  checkAnkiconnectInstalled: () => commands.checkAnkiconnectInstalled(),
  listRecentCaptures: () => commands.listRecentCaptures(50),
  deleteCapture: () => commands.deleteCapture(7),
  syncPendingCaptures: () => commands.syncPendingCaptures(),
  captureInProgress: () => commands.captureInProgress(),
  restartApp: () => commands.restartApp(),
}

// ---------------------------------------------------------------------------
// The events of ./events.ts and their payload types

const SUBSCRIBERS = {
  'translation-payload': onTranslationPayload,
  'capture-error': onCaptureError,
  'captures-changed': onCapturesChanged,
}
type EventName = keyof typeof SUBSCRIBERS

/** What a subscriber hands its handler; `null` for one that gets no payload. */
type Payload<S> = S extends (handler: (...args: infer A) => void) => unknown
  ? A extends [infer P]
    ? P
    : null
  : never

interface Payloads {
  PendingCapture: T.PendingCapture
  CaptureErrorPayload: T.CaptureErrorPayload
  null: null
}

const PAYLOADS = {
  'translation-payload': 'PendingCapture',
  'capture-error': 'CaptureErrorPayload',
  'captures-changed': 'null',
} as const satisfies Record<EventName, keyof Payloads>

type WrongPayloads = {
  [E in EventName]: Equals<
    Payloads[(typeof PAYLOADS)[E]],
    Payload<(typeof SUBSCRIBERS)[E]>
  > extends true
    ? never
    : E
}[EventName]
// A compile error here names the event whose handler gets another type than the backend sends.
const payloadsMatch: [WrongPayloads] extends [never] ? true : WrongPayloads = true

beforeEach(() => {
  resetTauri()
})

describe('the Rust shapes of the Automator', () => {
  it('match the TypeScript types at compile time', () => {
    expect([schemasMatchTypes, resultsMatch, payloadsMatch]).toEqual([true, true, true])
  })

  it('cover every type of ./types.ts, and nothing else', () => {
    expect(Object.keys(fixture.shapes).sort()).toEqual(Object.keys(SCHEMAS).sort())
  })

  it.each(Object.keys(SCHEMAS) as ShapeName[])(
    '%s: every sample has the TypeScript shape',
    (name) => {
      expect(samples[name].length).toBeGreaterThan(0)
      for (const value of samples[name]) expect(() => SCHEMAS[name].check(value)).not.toThrow()
    },
  )

  it('include every variant of the literal unions', () => {
    expect(samples.ScriptMetadata.map((value) => (value as T.ScriptMetadata).kind)).toEqual(
      expect.arrayContaining(['none', 'chinese']),
    )
    expect(new Set(samples.ProviderError.map((value) => (value as T.ProviderError).kind))).toEqual(
      new Set(['network', 'quota_exceeded', 'http', 'malformed', 'not_found', 'unsupported']),
    )
    expect(new Set(samples.CardField)).toEqual(new Set(CARD_FIELDS))
  })
})

describe('the commands of ./commands.ts', () => {
  it('are exactly the backend commands', () => {
    const wrappers = Object.keys(commands).filter((name) => name !== 'errorMessage')
    expect(wrappers.sort()).toEqual(fixture.commands.map((command) => command.function).sort())
    expect(Object.keys(CALLS).sort()).toEqual(wrappers)
  })

  it.each(fixture.commands)(
    '$function invokes $command with the arguments the backend reads',
    async ({ function: wrapper, command, args, returns }) => {
      onCommand(command, null)
      await CALLS[wrapper as Wrapper]()
      expect(invokeCalls).toHaveLength(1)
      expect(invokeCalls[0].cmd).toBe(command)
      expect(Object.keys(invokeCalls[0].args ?? {}).sort()).toEqual([...args].sort())
      expect(RETURNS[wrapper as Wrapper]).toBe(returns)
    },
  )
})

describe('docs/architecture.md § 8.2', () => {
  it('documents every command with the backend’s arguments and result type', () => {
    const byCommand = (a: { command: string }, b: { command: string }) =>
      a.command.localeCompare(b.command)
    expect(documentedCommands(architecture, '### 8.2 Commands').sort(byCommand)).toEqual(
      fixture.commands
        .map(({ command, args, returns }) => ({ command, args, returns }))
        .sort(byCommand),
    )
  })

  it('documents every event', () => {
    const events = /^Events: (.*?)\n\n/ms.exec(architecture)?.[1] ?? ''
    const documented = [...events.matchAll(/`([a-z-]+)` \(/g)].map(([, name]) => name)
    expect(documented).toEqual(fixture.events.map((event) => event.name))
  })
})

describe('the events of ./events.ts', () => {
  it('are exactly the events the backend emits, with the payload types it sends', () => {
    expect(fixture.events.map((event) => event.name).sort()).toEqual(
      Object.keys(SUBSCRIBERS).sort(),
    )
    for (const event of fixture.events) {
      expect(PAYLOADS[event.name as EventName]).toBe(event.payload)
    }
  })

  it.each(fixture.events)('$name reaches its handler', async ({ name, payload }) => {
    const handler = vi.fn()
    const unlisten = await SUBSCRIBERS[name as EventName](handler)
    const value = payload === 'null' ? null : sample(payload as 'PendingCapture')
    emitEvent(name, value)
    expect(handler).toHaveBeenCalledTimes(1)
    if (payload !== 'null') expect(handler).toHaveBeenCalledWith(value)
    unlisten()
  })
})

describe('the frontend reads every value the backend sends', () => {
  it('starts from the backend defaults and mappings', () => {
    expect(defaultConfig()).toEqual(samples.AppConfig[0])
    expect(appleStyleMapping()).toEqual(samples.FieldMapping[0])
    expect(chineseMapping()).toEqual(samples.FieldMapping[1])
  })

  it.each(samples.AppConfig.map((value, index) => [index, value]))(
    'the settings form keeps config %i and sends it back unchanged',
    (_index, value) => {
      const config = normalizeConfig(value)
      expect(config).toEqual(value)
      expect(toSettingsPayload(config)).toEqual(value)
    },
  )

  it('has words for every stage, failure, provider and origin, in both UI languages', () => {
    for (const value of samples.PipelineWarning) {
      expect(fixture.providers).toContain((value as T.PipelineWarning).error.provider)
    }
    for (const locale of ['en', 'de'] as const) {
      const has = (key: string) => i18n.global.te(key, locale)
      for (const stage of samples.PipelineStage) expect(has(`warnings.stage.${stage}`)).toBe(true)
      for (const value of samples.ProviderError) {
        expect(has(`warnings.kind.${(value as T.ProviderError).kind}`)).toBe(true)
      }
      for (const provider of fixture.providers) expect(has(`providers.${provider}`)).toBe(true)
      for (const origin of samples.CaptureOrigin) expect(has(`history.origin.${origin}`)).toBe(true)
    }
  })

  it('formats every pipeline warning without falling back to raw names', () => {
    const has = (key: string) => i18n.global.te(key, 'en')
    const t = (key: string, params?: Record<string, unknown>) =>
      i18n.global.t(key, params ?? {}, { locale: 'en' })
    for (const value of samples.PipelineWarning) {
      const warning = value as T.PipelineWarning
      const { text } = formatWarning(warning, t, has)
      expect(text.startsWith(`${t(`warnings.stage.${warning.stage}`)}: `)).toBe(true)
      expect(text).not.toContain(t('warnings.kind.other', { provider: '' }).trim())
    }
  })
})
