import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, ConsentState, SetupStatus } from '../api'
import {
  callsTo,
  commandNames,
  deferred,
  invokeCalls,
  onCommand,
  resetTauri,
} from '../test/fakeTauri'
import { useAnkiConnection, type AnkiConnectionOptions } from './useAnkiConnection'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const ONLINE: AnkiStatus = {
  online: true,
  version: 6,
  decks: ['Default', 'English::AutomatedVocabulary'],
  models: ['Basic', 'AppleStyleFinalV3'],
  error: null,
}
const OFFLINE: AnkiStatus = {
  online: false,
  version: null,
  decks: [],
  models: [],
  error: 'connection refused',
}
const NO_CONSENT: ConsentState = {
  anki_install_granted_at: null,
  ankiconnect_provision_granted_at: null,
}

function setup(overrides: Partial<SetupStatus> = {}): SetupStatus {
  return {
    anki_installed: true,
    ankiconnect_installed: true,
    platform_supported: true,
    anki_version: '24.06.3',
    consent: NO_CONSENT,
    ...overrides,
  }
}

/** The commands that may run without a click (§ 8.4: startup only checks). */
const CHECK_ONLY = new Set(['anki_status', 'get_setup_status'])

const scopes: ReturnType<typeof effectScope>[] = []

function create(options: AnkiConnectionOptions = {}) {
  const scope = effectScope()
  scopes.push(scope)
  const connection = scope.run(() =>
    useAnkiConnection({ pollIntervalMs: 100, maxPollAttempts: 3, ...options }),
  )
  if (!connection) throw new Error('scope did not run')
  return { connection, scope }
}

beforeEach(() => resetTauri())
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  vi.useRealTimers()
})

describe('resolving the state', () => {
  it('starts in checking and goes online when anki_status answers, without a disk check', async () => {
    onCommand('anki_status', ONLINE)
    const { connection } = create()
    expect(connection.state.value).toBe('checking')
    expect(await connection.check()).toBe('online')
    expect(connection.decks.value).toEqual(ONLINE.decks)
    expect(connection.models.value).toEqual(ONLINE.models)
    expect(commandNames()).toEqual(['anki_status'])
  })

  it.each([
    [{ anki_installed: false, ankiconnect_installed: false }, 'anki_missing'],
    [{ anki_installed: false, ankiconnect_installed: true }, 'anki_missing'],
    [{ anki_installed: true, ankiconnect_installed: false }, 'plugin_missing'],
    [{ anki_installed: true, ankiconnect_installed: true }, 'anki_closed'],
  ] as const)('offline with %o resolves to %s', async (disk, expected) => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup(disk))
    const { connection } = create()
    expect(await connection.check()).toBe(expected)
    expect(connection.state.value).toBe(expected)
    expect(connection.decks.value).toEqual([])
    expect(commandNames()).toEqual(['anki_status', 'get_setup_status'])
  })

  it('treats a failing anki_status as offline', async () => {
    onCommand('anki_status', () => {
      throw 'ipc failure'
    })
    onCommand('get_setup_status', setup({ ankiconnect_installed: false }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { connection } = create()
    expect(await connection.check()).toBe('plugin_missing')
    expect(connection.status.value).toBeNull()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('falls back to anki_closed and says why when the disk check fails', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', () => {
      throw 'permission denied'
    })
    const { connection } = create()
    expect(await connection.check()).toBe('anki_closed')
    expect(connection.feedback.value).toEqual({
      tone: 'error',
      key: 'anki.feedback.checkFailed',
      params: { error: 'permission denied' },
    })
  })

  it('never installs or launches anything on its own', async () => {
    onCommand('anki_status', OFFLINE)
    for (const disk of [
      setup({ anki_installed: false }),
      setup({ ankiconnect_installed: false }),
      setup(),
    ]) {
      onCommand('get_setup_status', disk)
      const { connection } = create()
      await connection.check()
      await connection.rescan()
    }
    expect(commandNames().every((cmd) => CHECK_ONLY.has(cmd))).toBe(true)
  })

  it('drops the result of an older check that finishes after a newer one', async () => {
    const slow = deferred<AnkiStatus>()
    let call = 0
    onCommand('anki_status', () => (++call === 1 ? slow.promise : ONLINE))
    onCommand('get_setup_status', setup())
    const { connection } = create()
    const older = connection.check()
    expect(await connection.check()).toBe('online')
    slow.resolve(OFFLINE)
    await older
    expect(connection.state.value).toBe('online')
  })
})

describe('rescan', () => {
  it('passes through checking and goes online once Anki answers (offline -> online)', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    const { connection } = create()
    await connection.check()
    expect(connection.state.value).toBe('anki_closed')

    onCommand('anki_status', ONLINE)
    const rescan = connection.rescan()
    expect(connection.state.value).toBe('checking')
    expect(connection.activity.value).toBe('rescan')
    await rescan
    expect(connection.state.value).toBe('online')
    expect(connection.activity.value).toBeNull()
    expect(connection.feedback.value).toEqual({ tone: 'success', key: 'anki.feedback.online' })
  })

  it('notices Anki being closed (online -> anki_closed)', async () => {
    onCommand('anki_status', ONLINE)
    const { connection } = create()
    await connection.check()
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    await connection.rescan()
    expect(connection.state.value).toBe('anki_closed')
    expect(connection.decks.value).toEqual([])
    expect(connection.feedback.value).toEqual({ tone: 'error', key: 'anki.feedback.stillOffline' })
  })

  it('notices the add-on disappearing (online -> plugin_missing)', async () => {
    onCommand('anki_status', ONLINE)
    const { connection } = create()
    await connection.check()
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup({ ankiconnect_installed: false }))
    await connection.rescan()
    expect(connection.state.value).toBe('plugin_missing')
  })

  it('ignores a second rescan while one is running', async () => {
    const pending = deferred<AnkiStatus>()
    onCommand('anki_status', () => pending.promise)
    const { connection } = create()
    const first = connection.rescan()
    await connection.rescan()
    pending.resolve(ONLINE)
    await first
    expect(callsTo('anki_status')).toHaveLength(1)
  })
})

describe('launch and polling', () => {
  beforeEach(() => vi.useFakeTimers())

  async function closedConnection() {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    onCommand('launch_anki_desktop', 'Launching Anki')
    const created = create()
    await created.connection.check()
    invokeCalls.length = 0
    return created
  }

  it('launches Anki, then polls anki_status until it answers', async () => {
    const { connection } = await closedConnection()
    let polls = 0
    onCommand('anki_status', () => (++polls >= 2 ? ONLINE : OFFLINE))

    const launch = connection.launch()
    await vi.advanceTimersByTimeAsync(0)
    expect(commandNames()).toEqual(['launch_anki_desktop'])
    expect(connection.activity.value).toBe('launch')
    expect(connection.feedback.value?.key).toBe('anki.feedback.launching')

    await vi.advanceTimersByTimeAsync(100)
    expect(polls).toBe(1)
    expect(connection.state.value).toBe('anki_closed')

    await vi.advanceTimersByTimeAsync(100)
    await launch
    expect(polls).toBe(2)
    expect(connection.state.value).toBe('online')
    expect(connection.activity.value).toBeNull()
    expect(connection.feedback.value).toEqual({ tone: 'success', key: 'anki.feedback.online' })
  })

  it('gives up after the last poll, re-checks and explains', async () => {
    const { connection } = await closedConnection()
    const launch = connection.launch()
    await vi.advanceTimersByTimeAsync(300)
    await launch
    expect(callsTo('anki_status')).toHaveLength(4)
    expect(callsTo('get_setup_status')).toHaveLength(1)
    expect(connection.state.value).toBe('anki_closed')
    expect(connection.activity.value).toBeNull()
    expect(connection.feedback.value).toEqual({ tone: 'error', key: 'anki.feedback.launchTimeout' })
  })

  it('reports a failed launch without polling', async () => {
    const { connection } = await closedConnection()
    onCommand('launch_anki_desktop', () => {
      throw 'Anki.exe not found'
    })
    await connection.launch()
    await vi.advanceTimersByTimeAsync(1000)
    expect(callsTo('anki_status')).toHaveLength(0)
    expect(connection.feedback.value).toEqual({
      tone: 'error',
      key: 'anki.feedback.launchFailed',
      params: { error: 'Anki.exe not found' },
    })
  })

  it('stops polling when its scope ends', async () => {
    const { connection, scope } = await closedConnection()
    void connection.launch()
    await vi.advanceTimersByTimeAsync(100)
    expect(callsTo('anki_status')).toHaveLength(1)
    scope.stop()
    await vi.advanceTimersByTimeAsync(1000)
    expect(callsTo('anki_status')).toHaveLength(1)
  })
})

describe('consent-gated installs', () => {
  const GRANTED_AT = '2026-10-31T12:00:00.000Z'

  async function connectionIn(disk: SetupStatus, options: AnkiConnectionOptions = {}) {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', disk)
    const created = create(options)
    await created.connection.check()
    invokeCalls.length = 0
    return created.connection
  }

  it('provision: opens the dialog first, then grants consent strictly before downloading', async () => {
    const onConsentChanged = vi.fn()
    const connection = await connectionIn(setup({ ankiconnect_installed: false }), {
      onConsentChanged,
    })
    onCommand('grant_consent', ({ action }: { action: string }) => ({
      ...NO_CONSENT,
      ankiconnect_provision_granted_at: action === 'ankiconnect_provision' ? GRANTED_AT : null,
    }))
    onCommand('auto_provision_ankiconnect', () => {
      onCommand('get_setup_status', setup())
      return 'Provisioned AnkiConnect.'
    })

    connection.requestProvision()
    expect(connection.pendingConsent.value).toBe('ankiconnect_provision')
    expect(commandNames()).toEqual([])

    await connection.confirmConsent()
    expect(commandNames()).toEqual([
      'grant_consent',
      'auto_provision_ankiconnect',
      'anki_status',
      'get_setup_status',
    ])
    expect(callsTo('grant_consent')[0].args).toEqual({ action: 'ankiconnect_provision' })
    expect(onConsentChanged).toHaveBeenCalledWith({
      ...NO_CONSENT,
      ankiconnect_provision_granted_at: GRANTED_AT,
    })
    expect(connection.pendingConsent.value).toBeNull()
    expect(connection.state.value).toBe('anki_closed')
    expect(connection.feedback.value).toEqual({ tone: 'success', key: 'anki.feedback.provisioned' })
  })

  it('cancel records nothing and downloads nothing', async () => {
    const connection = await connectionIn(setup({ ankiconnect_installed: false }))
    connection.requestProvision()
    connection.cancelConsent()
    await connection.confirmConsent()
    expect(connection.pendingConsent.value).toBeNull()
    expect(commandNames()).toEqual([])
    expect(connection.state.value).toBe('plugin_missing')
  })

  it('install: offered only when Anki is missing on a supported platform', async () => {
    const unsupported = await connectionIn(
      setup({ anki_installed: false, platform_supported: false }),
    )
    expect(unsupported.canInstallAnki.value).toBe(false)
    unsupported.requestInstall()
    expect(unsupported.pendingConsent.value).toBeNull()

    const installed = await connectionIn(setup({ ankiconnect_installed: false }))
    expect(installed.canInstallAnki.value).toBe(false)
    installed.requestInstall()
    expect(installed.pendingConsent.value).toBeNull()

    const missing = await connectionIn(setup({ anki_installed: false }))
    expect(missing.canInstallAnki.value).toBe(true)
    expect(missing.ankiVersion.value).toBe('24.06.3')
  })

  it('install: grants anki_install before download_and_install_anki, then re-checks', async () => {
    const onConsentChanged = vi.fn()
    const connection = await connectionIn(
      setup({ anki_installed: false, ankiconnect_installed: false }),
      { onConsentChanged },
    )
    onCommand('grant_consent', { ...NO_CONSENT, anki_install_granted_at: GRANTED_AT })
    onCommand('download_and_install_anki', () => {
      onCommand('get_setup_status', setup({ ankiconnect_installed: false }))
      return 'Installed Anki 24.06.3'
    })
    connection.requestInstall()
    expect(connection.pendingConsent.value).toBe('anki_install')

    const confirm = connection.confirmConsent()
    expect(connection.activity.value).toBe('install')
    await confirm
    expect(commandNames()).toEqual([
      'grant_consent',
      'download_and_install_anki',
      'anki_status',
      'get_setup_status',
    ])
    expect(callsTo('grant_consent')[0].args).toEqual({ action: 'anki_install' })
    expect(onConsentChanged).toHaveBeenCalledWith({
      ...NO_CONSENT,
      anki_install_granted_at: GRANTED_AT,
    })
    expect(connection.state.value).toBe('plugin_missing')
    expect(connection.activity.value).toBeNull()
  })

  it('never downloads when the consent could not be recorded', async () => {
    const connection = await connectionIn(setup({ ankiconnect_installed: false }))
    onCommand('grant_consent', () => {
      throw 'config.json is read-only'
    })
    onCommand('auto_provision_ankiconnect', 'should not run')
    connection.requestProvision()
    await connection.confirmConsent()
    expect(commandNames()).toEqual(['grant_consent'])
    expect(connection.feedback.value).toEqual({
      tone: 'error',
      key: 'anki.feedback.consentFailed',
      params: { error: 'config.json is read-only' },
    })
    expect(connection.state.value).toBe('plugin_missing')
  })

  it('reports a failed install and re-checks the setup', async () => {
    const connection = await connectionIn(setup({ ankiconnect_installed: false }))
    onCommand('grant_consent', { ...NO_CONSENT, ankiconnect_provision_granted_at: GRANTED_AT })
    onCommand('auto_provision_ankiconnect', () => {
      throw 'checksum mismatch for web.py'
    })
    connection.requestProvision()
    await connection.confirmConsent()
    expect(connection.feedback.value).toEqual({
      tone: 'error',
      key: 'anki.feedback.installFailed',
      params: { error: 'checksum mismatch for web.py' },
    })
    expect(connection.state.value).toBe('plugin_missing')
    expect(commandNames().slice(-2)).toEqual(['anki_status', 'get_setup_status'])
  })
})
