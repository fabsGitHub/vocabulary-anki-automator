import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AppConfig, Card, Detection, FieldCheck, PendingCapture, SubmitResult } from '../api'
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
  return {
    card: card(cardOverrides),
    warnings: [],
    profile_id: null,
    detection: null,
    filing: { deck: 'English::AutomatedVocabulary', model: 'AppleStyleFinalV3' },
    generation: 1,
    ...overrides,
  }
}

/** "message" read as French among English and French, unsure. */
function unsure(overrides: Partial<Detection> = {}): Detection {
  return {
    lang: 'fr',
    method: 'statistical',
    confident: false,
    script: 'latin',
    candidates: ['en', 'fr'],
    ranked: [
      { lang: 'fr', confidence: 0.61 },
      { lang: 'en', confidence: 0.39 },
    ],
    ...overrides,
  }
}

/** A capture of "message" read as French, with `detection`. */
function french(detection: Detection = unsure()): PendingCapture {
  return capture(
    { detection, filing: { deck: 'french', model: 'AppleStyleFinalV3' } },
    {
      front: 'message',
      back: 'Nachricht',
      example_source: '',
      example_target: '',
      notes: '',
      source_lang: 'fr',
    },
  )
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
  // Unknown commands reject; every shown capture is checked in the background.
  onCommand('check_pending_fields', null)
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

  it('ignores the Enter and Esc that WebKit reports as keyCode 229 while composing', async () => {
    await mountWindow(capture())
    press('Enter', { keyCode: 229 }, field('front'))
    press('Escape', { keyCode: 229 }, field('front'))
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
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
    await mountWindow(
      capture({ profile_id: 'zh-de', filing: { deck: 'Chinese::HSK', model: 'Chinese (HSK)' } }),
    )
    expect(wrapper?.text()).toContain('Hotkey: Chinese → German')
    expect(wrapper?.find('[data-test="deck"]').text()).toBe('Deck: Chinese::HSK')
    expect(wrapper?.find('[data-test="model"]').text()).toBe('Note type: Chinese (HSK)')
  })

  it('takes the deck and note type from the capture, not from the settings', async () => {
    onCommand(
      'get_current_settings',
      settings((config) => (config.anki.deck_name = 'Somewhere::Else')),
    )
    await mountWindow(capture({ filing: { deck: 'french', model: 'Basic' } }))
    expect(wrapper?.find('[data-test="deck"]').text()).toBe('Deck: french')
    expect(wrapper?.find('[data-test="model"]').text()).toBe('Note type: Basic')
    expect(wrapper?.text()).not.toContain('Somewhere::Else')
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

describe('the detected language', () => {
  const chips = () =>
    wrapper
      ?.find('[data-test="language-chips"]')
      .findAll('button')
      .map((chip) => ({
        text: chip.text(),
        pressed: chip.attributes('aria-pressed'),
        classes: chip.classes(),
        label: chip.attributes('aria-label'),
        title: chip.attributes('title'),
      }))

  it('offers one chip per candidate, the guess marked as unsure', async () => {
    await mountWindow(french())

    const group = wrapper?.find('[data-test="language-chips"]')
    expect(group?.attributes('role')).toBe('group')
    expect(group?.attributes('aria-label')).toBe('Language of the text')
    expect(chips()).toEqual([
      {
        text: 'English',
        pressed: 'false',
        classes: ['badge', 'badge-muted'],
        label: 'Capture as English',
        title: 'Capture as English',
      },
      {
        text: 'French?',
        pressed: 'true',
        classes: ['badge', 'badge-warning'],
        label: 'Capture as French',
        title: 'Capture as French',
      },
    ])
    expect(wrapper?.find('[data-test="language-en"]').exists()).toBe(true)
    expect(wrapper?.find('[data-test="target-lang"]').text()).toBe('→ German')
    expect(wrapper?.find('[data-test="language-pair"]').exists()).toBe(false)
  })

  it('shows a confident guess as a plain chip and no hint', async () => {
    await mountWindow(french(unsure({ confident: true, ranked: [] })))
    expect(chips()?.[1]).toMatchObject({ text: 'French', pressed: 'true', classes: ['badge'] })
    expect(wrapper?.find('[data-test="language-hint"]').exists()).toBe(false)
  })

  it('keeps the language pair without detection or without a choice', async () => {
    for (const detection of [
      null,
      unsure({ lang: 'en', method: 'single', confident: true, candidates: ['en'] }),
    ]) {
      await mountWindow(capture({ detection }))
      expect(wrapper?.find('[data-test="language-chips"]').exists()).toBe(false)
      expect(wrapper?.find('[data-test="language-pair"]').text()).toBe('English → German')
      wrapper?.unmount()
    }
    wrapper = null
  })

  it('explains an unsure guess, a tie-break and a fallback', async () => {
    const hint = () => wrapper?.find('[data-test="language-hint"]')
    const cases: [Detection, string][] = [
      [unsure(), 'This could be more than one of your languages. It was read as French.'],
      [
        unsure({ lang: 'en', method: 'tie_break' }),
        'This could be more than one of your languages. It was read as English.',
      ],
      [
        unsure({ lang: 'en', method: 'fallback', script: 'latin', ranked: [] }),
        'None of your languages fits this text. It was read as English.',
      ],
      [
        unsure({ lang: 'en', method: 'fallback', script: 'none', ranked: [] }),
        'None of your languages fits this text. It was read as English.',
      ],
      ...(['kana', 'hangul', 'cyrillic', 'other'] as const).map((script): [Detection, string] => [
        unsure({ lang: 'en', method: 'fallback', script, ranked: [] }),
        'The text is written in a script none of your languages uses. It was read as English.',
      ]),
    ]
    for (const [detection, text] of cases) {
      await mountWindow(french(detection))
      expect(hint()?.attributes('role'), detection.method).toBe('status')
      expect(hint()?.text(), `${detection.method}/${detection.script}`).toContain(text)
      wrapper?.unmount()
    }
    wrapper = null
  })

  it('leaves Enter on a chip to the chip', async () => {
    onCommand('execute_card_generation', SYNCED)
    await mountWindow(french())
    press('Enter', {}, wrapper?.find('[data-test="language-en"]').element)
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
  })
})

describe('switching the language', () => {
  /** What `rerun_pending_capture` returns for "message" captured as English. */
  function english(front = 'message'): PendingCapture {
    return capture(
      {
        detection: unsure({ lang: 'en', method: 'chosen', confident: true }),
        filing: { deck: 'english', model: 'AppleStyleFinalV3' },
      },
      { front, back: 'Nachricht', example_source: `An example with ${front}.`, notes: '' },
    )
  }

  const chip = (code: string) => wrapper?.find(`[data-test="language-${code}"]`)

  it('captures the edited front again in the chosen language and shows the result', async () => {
    onCommand('rerun_pending_capture', english('messages'))
    await mountWindow(french())
    await wrapper?.find('[data-test="front"]').setValue('messages')

    await chip('en')?.trigger('click')
    await flushPromises()

    const calls = callsTo('rerun_pending_capture')
    expect(calls).toHaveLength(1)
    expect(calls[0].args).toEqual({ sourceLang: 'en', text: 'messages', generation: 1 })
    expect(field('front').value).toBe('messages')
    expect(field('example-source').value).toBe('An example with messages.')
    expect(chip('en')?.attributes('aria-pressed')).toBe('true')
    expect(chip('en')?.text()).toBe('English')
    expect(wrapper?.find('[data-test="deck"]').text()).toBe('Deck: english')
    expect(wrapper?.find('[data-test="language-hint"]').exists()).toBe(false)
  })

  it('shows the switched card once when the event arrives before the result', async () => {
    onCommand('rerun_pending_capture', () => {
      emitEvent('translation-payload', english())
      return english()
    })
    await mountWindow(french())

    await chip('en')?.trigger('click')
    await flushPromises()

    expect(field('front').value).toBe('message')
    expect(chip('en')?.attributes('aria-pressed')).toBe('true')
    expect(callsTo('check_pending_fields'), 'the switched card is checked once').toHaveLength(2)
    expect(document.activeElement).toBe(chip('en')?.element)
  })

  it('announces the switch and blocks the card until it is done', async () => {
    const rerun = deferred<PendingCapture>()
    onCommand('rerun_pending_capture', () => rerun.promise)
    onCommand('execute_card_generation', SYNCED)
    onCommand('discard_pending_payload', null)
    await mountWindow(french())

    await chip('en')?.trigger('click')
    expect(chip('en')?.text()).toBe('Fetching the card in English…')
    expect(chip('en')?.attributes('aria-pressed')).toBe('true')
    expect(chip('fr')?.attributes('aria-pressed')).toBe('false')
    expect(wrapper?.find('[data-test="switching"]').attributes('role')).toBe('status')
    for (const name of ['approve', 'discard', 'language-en', 'language-fr']) {
      expect(wrapper?.find(`[data-test="${name}"]`).attributes('disabled'), name).toBeDefined()
    }
    expect(field('front').readOnly).toBe(true)
    await wrapper?.find('[data-test="approve"]').trigger('click')
    press('Enter')
    press('Escape')
    await chip('fr')?.trigger('click')
    await flushPromises()
    expect(callsTo('execute_card_generation')).toHaveLength(0)
    expect(callsTo('discard_pending_payload')).toHaveLength(0)
    expect(callsTo('rerun_pending_capture')).toHaveLength(1)

    rerun.resolve(english())
    await flushPromises()
    expect(wrapper?.find('[data-test="switching"]').exists()).toBe(false)
    expect(wrapper?.find('[data-test="approve"]').attributes('disabled')).toBeUndefined()
    expect(document.activeElement).toBe(chip('en')?.element)
  })

  it('leaves the focus on the empty back after a switch', async () => {
    const switched = english()
    switched.card.back = ''
    onCommand('rerun_pending_capture', switched)
    await mountWindow(french())

    await chip('en')?.trigger('click')
    await flushPromises()

    expect(document.activeElement).toBe(field('back'))
  })

  it('ignores a late result when a newer capture arrived meanwhile', async () => {
    const rerun = deferred<PendingCapture>()
    onCommand('rerun_pending_capture', () => rerun.promise)
    await mountWindow(french())
    await chip('en')?.trigger('click')

    emitEvent(
      'translation-payload',
      capture({}, { front: 'house', back: 'Haus', captured_at: '2026-10-31T12:09:00.000Z' }),
    )
    await flushPromises()
    rerun.resolve(english())
    await flushPromises()

    expect(field('front').value).toBe('house')
    expect(wrapper?.find('[data-test="language-chips"]').exists()).toBe(false)
  })

  it('names the language and the error when the switch fails', async () => {
    onCommand('rerun_pending_capture', () => {
      throw 'The card was saved or discarded in the meantime.'
    })
    await mountWindow(french())

    await chip('en')?.trigger('click')
    await flushPromises()

    expect(wrapper?.find('[data-test="approval-error"]').text()).toBe(
      'Could not switch to English: The card was saved or discarded in the meantime.',
    )
    expect(field('front').value).toBe('message')
    expect(chip('fr')?.text()).toBe('French?')
    expect(wrapper?.find('[data-test="approve"]').attributes('disabled')).toBeUndefined()
  })

  it('does not fetch the card again for the language it already is in', async () => {
    await mountWindow(french())
    await chip('fr')?.trigger('click')
    await flushPromises()
    expect(callsTo('rerun_pending_capture')).toHaveLength(0)
  })
})

describe('the field check', () => {
  const LOST: FieldCheck = {
    status: 'checked',
    model: 'AppleStyleFinalV3',
    auto_matched: false,
    lost: [
      { kind: 'missing_field', anki_field: 'Context Sentence', source: 'example' },
      { kind: 'unmapped', source: 'pinyin' },
      { kind: 'audio', audio_field: 'Front Audio' },
      { kind: 'audio', audio_field: null },
    ],
  }

  const lostLines = () =>
    wrapper?.findAll('[data-test="lost-value"]').map((item) => item.text()) ?? []

  it('lists what the note type would drop, and approving stays possible', async () => {
    onCommand('check_pending_fields', LOST)
    await mountWindow(capture())

    const notice = wrapper?.find('[data-test="fields-lost"]')
    expect(notice?.attributes('role')).toBe('status')
    expect(notice?.text()).toContain('Not everything would reach Anki:')
    expect(lostLines()).toEqual([
      'Example and translation (two lines): the note type “AppleStyleFinalV3” has no field “Context Sentence”.',
      'Pinyin: no field of “AppleStyleFinalV3” is filled with it.',
      'Pronunciation: the note type “AppleStyleFinalV3” has no field “Front Audio”.',
      'Pronunciation: no field of “AppleStyleFinalV3” is chosen for the audio.',
    ])
    expect(notice?.text()).toContain('under “My languages” or “Anki connection”')
    expect(wrapper?.find('[data-test="approve"]').attributes('disabled')).toBeUndefined()
  })

  it('says nothing when everything arrives, Anki is away or nothing was checked', async () => {
    for (const check of [
      { status: 'checked', model: 'AppleStyleFinalV3', auto_matched: true, lost: [] },
      { status: 'unchecked' },
      null,
    ] satisfies (FieldCheck | null)[]) {
      onCommand('check_pending_fields', check)
      await mountWindow(capture())
      expect(wrapper?.find('[data-test="fields-lost"]').exists(), JSON.stringify(check)).toBe(false)
      wrapper?.unmount()
    }
    wrapper = null
  })

  it('names the error when the note type could not be checked', async () => {
    onCommand('check_pending_fields', {
      status: 'failed',
      model: 'Nope',
      error: 'model was not found: Nope',
    } satisfies FieldCheck)
    await mountWindow(capture())
    expect(wrapper?.find('[data-test="fields-lost"]').text()).toBe(
      'The note type could not be checked: model was not found: Nope',
    )
  })

  it('shows no warning when the check itself fails', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    onCommand('check_pending_fields', () => {
      throw 'command check_pending_fields not found'
    })
    await mountWindow(capture())
    expect(wrapper?.find('[data-test="approval-card"]').exists()).toBe(true)
    expect(wrapper?.find('[data-test="fields-lost"]').exists()).toBe(false)
  })

  it('checks each shown capture once, not its re-deliveries', async () => {
    await mountWindow(capture())
    expect(callsTo('check_pending_fields')).toHaveLength(1)
    emitEvent('translation-payload', capture())
    await flushPromises()
    expect(callsTo('check_pending_fields')).toHaveLength(1)
    emitEvent('translation-payload', capture({}, { front: 'house', back: 'Haus' }))
    await flushPromises()
    expect(callsTo('check_pending_fields')).toHaveLength(2)
  })

  it('drops the answer for a capture that was replaced meanwhile', async () => {
    const slow = deferred<FieldCheck | null>()
    onCommand('check_pending_fields', () => slow.promise)
    await mountWindow(capture())
    onCommand('check_pending_fields', null)
    emitEvent('translation-payload', capture({}, { front: 'house', back: 'Haus' }))
    await flushPromises()

    slow.resolve(LOST)
    await flushPromises()

    expect(field('front').value).toBe('house')
    expect(wrapper?.find('[data-test="fields-lost"]').exists()).toBe(false)
  })
})
