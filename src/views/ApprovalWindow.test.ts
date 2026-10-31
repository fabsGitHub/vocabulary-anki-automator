import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AppConfig, Card, PendingCapture, SubmitResult } from '../api'
import { defaultConfig } from '../composables/useConfig'
import { i18n, setLocale } from '../i18n'
import {
  callsTo,
  coreModule,
  deferred,
  emitEvent,
  eventModule,
  hideWindow,
  listenerCount,
  onCommand,
  resetTauri,
  windowState,
} from '../test/fakeTauri'
import ApprovalWindow from './ApprovalWindow.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('../test/fakeTauri')).eventModule)
vi.mock('@tauri-apps/api/window', async () => (await import('../test/fakeTauri')).windowModule)

function card(overrides: Partial<Card> = {}): Card {
  return {
    front: 'serendipity',
    back: 'glücklicher Zufall',
    example_source: 'It was pure serendipity.',
    example_target: 'Es war reiner Zufall.',
    notes: 'Rare word.',
    tags: ['vocab-automator'],
    source_lang: 'en',
    target_lang: 'de',
    captured_at: '2026-10-31T12:00:00.123Z',
    audio_front: '//uQAA==',
    script_metadata: { kind: 'none' },
    ...overrides,
  }
}

function capture(
  overrides: Partial<PendingCapture> = {},
  cardOverrides: Partial<Card> = {},
): PendingCapture {
  return { card: card(cardOverrides), warnings: [], profile_id: null, ...overrides }
}

const SYNCED: SubmitResult = {
  capture_id: 7,
  synced_to_anki: true,
  note_ids: [11, 12],
  anki_error: null,
}
const OFFLINE: SubmitResult = {
  capture_id: 7,
  synced_to_anki: false,
  note_ids: [],
  anki_error: 'could not reach AnkiConnect: connection refused',
}

function settings(edit: (config: AppConfig) => void = () => {}): AppConfig {
  const config = defaultConfig()
  edit(config)
  return config
}

let wrapper: VueWrapper | null = null

/** Mounts the window; `pending` (unless omitted) is what `get_pending_payload` returns. */
async function mountWindow(pending?: PendingCapture | null): Promise<VueWrapper> {
  if (pending !== undefined) onCommand('get_pending_payload', pending)
  wrapper = mount(ApprovalWindow, { attachTo: document.body, global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}

function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const element = wrapper?.find(`[data-test="${name}"]`)
  if (!element?.exists()) throw new Error(`no ${name} field`)
  return element.element as HTMLInputElement | HTMLTextAreaElement
}

function press(
  key: string,
  init: KeyboardEventInit = {},
  target: EventTarget = document.body,
): void {
  target.dispatchEvent(
    new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }),
  )
}

function submittedCard(index = 0): Card {
  const call = callsTo('execute_card_generation')[index]
  expect(Object.keys(call.args ?? {})).toEqual(['card'])
  return call.args?.card as Card
}

beforeEach(() => {
  resetTauri()
  windowState.label = 'preview'
  onCommand('get_current_settings', settings())
  setLocale('en')
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  vi.restoreAllMocks()
})

describe('receiving a capture', () => {
  it('shows a capture pushed by translation-payload', async () => {
    await mountWindow(null)
    expect(wrapper?.find('[data-test="waiting"]').exists()).toBe(true)

    emitEvent('translation-payload', capture())
    await flushPromises()
    expect(field('front').value).toBe('serendipity')
    expect(field('back').value).toBe('glücklicher Zufall')
    expect(field('example-source').value).toBe('It was pure serendipity.')
    expect(field('example-target').value).toBe('Es war reiner Zufall.')
    expect(field('notes').value).toBe('Rare word.')
    expect(wrapper?.find('[data-test="language-pair"]').text()).toBe('English → German')
  })

  it('fetches the capture that was already pending, after it started listening', async () => {
    await mountWindow(capture())
    expect(field('front').value).toBe('serendipity')
    const listenedAt = Math.min(...eventModule.listen.mock.invocationCallOrder)
    const fetchIndex = coreModule.invoke.mock.calls.findIndex(
      ([cmd]) => cmd === 'get_pending_payload',
    )
    expect(fetchIndex).toBeGreaterThanOrEqual(0)
    expect(listenedAt).toBeLessThan(coreModule.invoke.mock.invocationCallOrder[fetchIndex])
  })

  it('lets an event that arrives during the fetch win over the older pending capture', async () => {
    const slow = deferred<PendingCapture | null>()
    onCommand('get_pending_payload', () => slow.promise)
    await mountWindow()
    emitEvent('translation-payload', capture({}, { front: 'newer' }))
    slow.resolve(capture({}, { front: 'older' }))
    await flushPromises()
    expect(field('front').value).toBe('newer')
  })

  it('replaces the form with a newer capture but keeps edits on a re-delivery of the same one', async () => {
    await mountWindow(capture())
    await wrapper?.find('[data-test="back"]').setValue('edited')
    emitEvent('translation-payload', capture())
    await flushPromises()
    expect(field('back').value).toBe('edited')

    emitEvent(
      'translation-payload',
      capture({}, { front: 'house', back: 'Haus', captured_at: '2026-10-31T12:05:00.000Z' }),
    )
    await flushPromises()
    expect(field('front').value).toBe('house')
    expect(field('back').value).toBe('Haus')
  })

  it('shows an updated payload for the same capture instead of ignoring it', async () => {
    await mountWindow(capture({}, { back: '' }))
    emitEvent('translation-payload', capture({}, { back: 'glücklicher Zufall' }))
    await flushPromises()
    expect(field('back').value).toBe('glücklicher Zufall')
  })

  it('stops listening when the window unmounts', async () => {
    await mountWindow(null)
    expect(listenerCount('translation-payload')).toBe(1)
    expect(listenerCount('capture-error')).toBe(1)
    wrapper?.unmount()
    wrapper = null
    expect(listenerCount('translation-payload')).toBe(0)
    expect(listenerCount('capture-error')).toBe(0)
  })
})

describe('approving', () => {
  it('sends the edited card, trimmed, to execute_card_generation', async () => {
    onCommand('execute_card_generation', SYNCED)
    await mountWindow(capture())
    await wrapper?.find('[data-test="front"]').setValue('  serendipitous ')
    await wrapper?.find('[data-test="back"]').setValue('zufällig')
    await wrapper?.find('[data-test="example-source"]').setValue('A serendipitous find.')
    await wrapper?.find('[data-test="example-target"]').setValue('Ein zufälliger Fund.')
    await wrapper?.find('[data-test="notes"]').setValue('')

    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()
    expect(submittedCard()).toEqual({
      ...card(),
      front: 'serendipitous',
      back: 'zufällig',
      example_source: 'A serendipitous find.',
      example_target: 'Ein zufälliger Fund.',
      notes: '',
    })
  })

  it('hides the window once Anki took the card', async () => {
    onCommand('execute_card_generation', SYNCED)
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()
    expect(hideWindow).toHaveBeenCalledTimes(1)
    expect(wrapper?.find('[data-test="waiting"]').exists()).toBe(true)
  })

  it('stays open after an offline save, says so, and only Close is left', async () => {
    onCommand('execute_card_generation', OFFLINE)
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()

    expect(hideWindow).not.toHaveBeenCalled()
    const notice = wrapper?.find('[data-test="saved-offline"]')
    expect(notice?.text()).toContain('Saved to the history, but Anki is unreachable')
    expect(notice?.text()).toContain('connection refused')
    expect(wrapper?.find('[data-test="approve"]').exists()).toBe(false)
    expect(field('front').readOnly).toBe(true)

    press('Enter')
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(1)
    expect(hideWindow).toHaveBeenCalledTimes(1)
  })

  it('Close hides the window after an offline save', async () => {
    onCommand('execute_card_generation', OFFLINE)
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()
    await wrapper?.find('[data-test="close"]').trigger('click')
    await flushPromises()
    expect(hideWindow).toHaveBeenCalledTimes(1)
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
  })

  it('submits once, however often Approve and Enter are pressed', async () => {
    const pending = deferred<SubmitResult>()
    onCommand('execute_card_generation', () => pending.promise)
    await mountWindow(capture())
    const approve = wrapper?.find('[data-test="approve"]')
    await approve?.trigger('click')
    await approve?.trigger('click')
    press('Enter')
    press('Enter', {}, field('back'))
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(1)
    expect((approve?.element as HTMLButtonElement).disabled).toBe(true)
    pending.resolve(SYNCED)
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(1)
  })

  it('shows the error and lets the user retry when saving fails', async () => {
    let attempts = 0
    onCommand('execute_card_generation', () => {
      if (++attempts === 1) throw 'database is locked'
      return SYNCED
    })
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()
    expect(wrapper?.find('[data-test="approval-error"]').text()).toContain('database is locked')
    expect(hideWindow).not.toHaveBeenCalled()
    await wrapper?.find('[data-test="approve"]').trigger('click')
    await flushPromises()
    expect(attempts).toBe(2)
    expect(hideWindow).toHaveBeenCalledTimes(1)
  })

  it('refuses an empty front', async () => {
    onCommand('execute_card_generation', SYNCED)
    await mountWindow(capture())
    await wrapper?.find('[data-test="front"]').setValue('   ')
    expect((wrapper?.find('[data-test="approve"]').element as HTMLButtonElement).disabled).toBe(
      true,
    )
    press('Enter')
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
    expect(wrapper?.find('[data-test="approval-error"]').text()).toContain('front cannot be empty')
  })

  it('does not hide a newer capture when the previous card finishes saving', async () => {
    const pending = deferred<SubmitResult>()
    onCommand('execute_card_generation', () => pending.promise)
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    emitEvent(
      'translation-payload',
      capture({}, { front: 'house', captured_at: '2026-10-31T12:09:00.000Z' }),
    )
    pending.resolve(SYNCED)
    await flushPromises()
    expect(hideWindow).not.toHaveBeenCalled()
    expect(field('front').value).toBe('house')
  })
})

describe('discarding', () => {
  it('drops the capture through discard_pending_payload', async () => {
    onCommand('discard_pending_payload', null)
    await mountWindow(capture())
    await wrapper?.find('[data-test="discard"]').trigger('click')
    await flushPromises()
    expect(callsTo('discard_pending_payload')).toHaveLength(1)
    expect(callsTo('execute_card_generation')).toHaveLength(0)
    expect(wrapper?.find('[data-test="waiting"]').exists()).toBe(true)
  })

  it('cannot discard a card while it is being saved', async () => {
    const pending = deferred<SubmitResult>()
    onCommand('execute_card_generation', () => pending.promise)
    onCommand('discard_pending_payload', null)
    await mountWindow(capture())
    await wrapper?.find('[data-test="approve"]').trigger('click')
    press('Escape')
    await flushPromises()
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
    pending.resolve(SYNCED)
    await flushPromises()
  })
})

describe('keyboard', () => {
  beforeEach(() => {
    onCommand('execute_card_generation', SYNCED)
    onCommand('discard_pending_payload', null)
  })

  it('Enter approves from anywhere in the window', async () => {
    await mountWindow(capture())
    press('Enter')
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(1)
  })

  it('Enter in a text field approves the edited value', async () => {
    await mountWindow(capture())
    await wrapper?.find('[data-test="back"]').setValue('Zufall')
    press('Enter', {}, field('back'))
    await flushPromises()
    expect(submittedCard().back).toBe('Zufall')
  })

  it('Shift+Enter and Alt+Enter do not approve', async () => {
    await mountWindow(capture())
    press('Enter', { shiftKey: true }, field('example-source'))
    press('Enter', { altKey: true })
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
  })

  it('ignores Enter while an input method is composing (pinyin keyboards)', async () => {
    await mountWindow(capture())
    press('Enter', { isComposing: true }, field('front'))
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
  })

  it('leaves Enter on a focused button to that button', async () => {
    await mountWindow(capture())
    const discard = wrapper?.find('[data-test="discard"]').element as HTMLButtonElement
    press('Enter', {}, discard)
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
  })

  it('Escape discards', async () => {
    await mountWindow(capture())
    press('Escape')
    await flushPromises()
    expect(callsTo('discard_pending_payload')).toHaveLength(1)
    expect(callsTo('execute_card_generation')).toHaveLength(0)
  })

  it('Escape after an offline save closes instead of discarding', async () => {
    onCommand('execute_card_generation', OFFLINE)
    await mountWindow(capture())
    press('Enter')
    await flushPromises()
    press('Escape')
    await flushPromises()
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
    expect(hideWindow).toHaveBeenCalledTimes(1)
  })

  it('stops handling keys once unmounted', async () => {
    await mountWindow(capture())
    wrapper?.unmount()
    wrapper = null
    press('Enter')
    press('Escape')
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
  })
})

describe('what the window shows', () => {
  it('turns pipeline warnings into sentences, with the MyMemory quota explained', async () => {
    await mountWindow(
      capture({
        warnings: [
          {
            stage: 'translate',
            error: {
              kind: 'quota_exceeded',
              provider: 'mymemory',
              message: 'NEXT AVAILABLE IN 10 HOURS',
            },
          },
          {
            stage: 'tts',
            error: { kind: 'network', provider: 'google_tts', message: 'timed out' },
          },
        ],
      }),
    )
    const warnings = wrapper?.findAll('[data-test="warning"]') ?? []
    expect(warnings).toHaveLength(2)
    expect(warnings[0].text()).toContain('Translation:')
    expect(warnings[0].text()).toContain('daily translation limit is used up')
    expect(warnings[0].text()).toContain('contact e-mail')
    expect(warnings[0].text()).toContain('NEXT AVAILABLE IN 10 HOURS')
    expect(warnings[1].text()).toContain(
      'Pronunciation: Google Text-to-Speech could not be reached',
    )
    expect(wrapper?.text()).not.toContain('quota_exceeded')
  })

  it('shows no warning box for a clean capture', async () => {
    await mountWindow(capture())
    expect(wrapper?.find('[data-test="warnings"]').exists()).toBe(false)
  })

  it('shows pinyin and the HSK level for Chinese cards only', async () => {
    await mountWindow(
      capture(
        {},
        {
          front: '学习',
          back: 'lernen',
          source_lang: 'zh-CN',
          script_metadata: {
            kind: 'chinese',
            pinyin: 'xué xí',
            hsk_level: 1,
            script_variant: 'simplified',
          },
        },
      ),
    )
    expect(wrapper?.find('[data-test="pinyin"]').text()).toBe('xué xí')
    expect(wrapper?.find('[data-test="hsk"]').text()).toBe('HSK 1')

    emitEvent(
      'translation-payload',
      capture(
        {},
        {
          front: '电脑',
          captured_at: '2026-10-31T12:01:00.000Z',
          script_metadata: {
            kind: 'chinese',
            pinyin: 'diàn nǎo',
            hsk_level: null,
            script_variant: 'simplified',
          },
        },
      ),
    )
    await flushPromises()
    expect(wrapper?.find('[data-test="hsk"]').text()).toBe('Not in the HSK list')

    emitEvent('translation-payload', capture({}, { captured_at: '2026-10-31T12:02:00.000Z' }))
    await flushPromises()
    expect(wrapper?.find('[data-test="chinese"]').exists()).toBe(false)
  })

  it('plays the captured pronunciation', async () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined)
    vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {})
    await mountWindow(capture())
    await wrapper?.find('[data-test="play-audio"]').trigger('click')
    expect(play).toHaveBeenCalledTimes(1)
    expect((play.mock.contexts[0] as HTMLAudioElement).src).toBe('data:audio/mpeg;base64,//uQAA==')
  })

  it('offers no playback without audio', async () => {
    await mountWindow(capture({}, { audio_front: null }))
    expect(wrapper?.find('[data-test="play-audio"]').exists()).toBe(false)
  })

  it('names the hotkey profile and the deck the card goes to', async () => {
    onCommand(
      'get_current_settings',
      settings((config) =>
        config.hotkey_profiles.push({
          id: 'zh-de',
          label: 'Chinese → German',
          shortcut: 'CommandOrControl+Shift+H',
          source_lang: 'zh-CN',
          target_lang: 'de',
          deck_name: 'Chinese::HSK',
        }),
      ),
    )
    await mountWindow(capture({ profile_id: 'zh-de' }))
    expect(wrapper?.text()).toContain('Hotkey: Chinese → German')
    expect(wrapper?.text()).toContain('Deck: Chinese::HSK')
  })

  it('uses the UI language from the settings', async () => {
    onCommand(
      'get_current_settings',
      settings((config) => (config.ui_lang = 'de')),
    )
    await mountWindow(capture())
    expect(wrapper?.find('[data-test="approve"]').text()).toBe('Zu Anki hinzufügen')
    expect(wrapper?.find('[data-test="discard"]').text()).toBe('Verwerfen')
  })

  it('shows capture errors sent while it is open', async () => {
    await mountWindow(capture())
    emitEvent('capture-error', {
      message: 'the text is 120 characters long; captures are limited to 100',
    })
    await flushPromises()
    expect(wrapper?.find('[data-test="capture-error"]').text()).toContain('limited to 100')
  })
})
