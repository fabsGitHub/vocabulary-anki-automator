import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as fx from '../test/fixtures'
import { TauriAdapter } from './tauri'
import type { ExerciseAnswer, PlatformAdapter } from './types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const invokeMock = vi.mocked(invoke)

type Call = {
  /** Calls the method under test. */
  run: (adapter: PlatformAdapter) => Promise<unknown>
  command: string
  /** The argument object `invoke` must get; `undefined` for commands without arguments. */
  args?: Record<string, unknown>
  /** What the backend answers, and what the method must resolve to. */
  reply: unknown
  /** What the method resolves to when it differs from `reply` (`null` -> `undefined`). */
  resolves?: unknown
}

const exercise = fx.multipleChoice()
const answer: ExerciseAnswer = { kind: 'choice', index: 1 }
const settings = fx.settings()

/**
 * Every PlatformAdapter method, with the command and camelCase args of
 * docs/architecture.md § 10. The type makes a method missing from this table
 * a compile error.
 */
const CALLS: Record<Exclude<keyof PlatformAdapter, 'capabilities'>, Call> = {
  getDailyPromptState: {
    run: (a) => a.getDailyPromptState(),
    command: 'get_daily_prompt_state',
    reply: fx.promptState({ needs_prompt: true }),
  },
  submitDailyQuota: {
    run: (a) => a.submitDailyQuota(7),
    command: 'submit_daily_quota',
    args: { newCount: 7 },
    reply: fx.session({ requested_new_count: 7 }),
  },
  dismissDailyPrompt: {
    run: (a) => a.dismissDailyPrompt(),
    command: 'dismiss_daily_prompt',
    reply: fx.session({ requested_new_count: 0 }),
  },
  reopenDailyPrompt: {
    run: (a) => a.reopenDailyPrompt(),
    command: 'reopen_daily_prompt',
    reply: null,
    resolves: undefined,
  },
  getDailyQueue: {
    run: (a) => a.getDailyQueue(),
    command: 'get_daily_queue',
    reply: [{ kind: 'new', hanzi: '学', curriculum_order: 1 }],
  },
  startNewItem: {
    run: (a) => a.startNewItem('学习'),
    command: 'start_new_item',
    args: { hanzi: '学习' },
    reply: 12,
  },
  startNewCapture: {
    run: (a) => a.startNewCapture(14),
    command: 'start_new_capture',
    args: { cardId: 14 },
    reply: 14,
  },
  getCard: {
    run: (a) => a.getCard(12),
    command: 'get_card',
    args: { cardId: 12 },
    reply: fx.capture({ id: 12 }),
  },
  previewRatings: {
    run: (a) => a.previewRatings(12),
    command: 'preview_ratings',
    args: { cardId: 12 },
    reply: fx.previews(),
  },
  submitReviewRating: {
    run: (a) => a.submitReviewRating(12, 3),
    command: 'submit_review_rating',
    args: { cardId: 12, rating: 3 },
    reply: fx.reviewResult(),
  },
  listLearnedCards: {
    run: (a) => a.listLearnedCards(),
    command: 'list_learned_cards',
    reply: [fx.learnedCard()],
  },
  nextExercise: {
    run: (a) => a.nextExercise(12, 'sentence_builder', 4_000_000_000),
    command: 'next_exercise',
    args: { cardId: 12, kind: 'sentence_builder', seed: 4_000_000_000 },
    reply: fx.sentenceBuilder(),
  },
  checkExercise: {
    run: (a) => a.checkExercise(exercise, answer),
    command: 'check_exercise',
    args: { exercise, answer },
    reply: true,
  },
  recordExerciseResult: {
    run: (a) => a.recordExerciseResult(12, 'listening_choice', false),
    command: 'record_exercise_result',
    args: { cardId: 12, kind: 'listening_choice', correct: false },
    reply: 0,
  },
  listLessons: {
    run: (a) => a.listLessons(),
    command: 'list_lessons',
    reply: [fx.lesson()],
  },
  completeLesson: {
    run: (a) => a.completeLesson('lesson-11'),
    command: 'complete_lesson',
    args: { lessonId: 'lesson-11' },
    reply: 20,
  },
  getStats: {
    run: (a) => a.getStats(),
    command: 'get_stats',
    reply: fx.stats(),
  },
  getCardAudio: {
    run: (a) => a.getCardAudio(12),
    command: 'get_card_audio',
    args: { cardId: 12 },
    reply: 'SUQz',
  },
  synthesizeAudio: {
    run: (a) => a.synthesizeAudio('我在学习中文。', 'zh-CN'),
    command: 'synthesize_audio',
    args: { text: '我在学习中文。', lang: 'zh-CN' },
    reply: null,
  },
  getSettings: {
    run: (a) => a.getSettings(),
    command: 'get_settings',
    reply: settings,
  },
  saveSettings: {
    run: (a) => a.saveSettings(settings),
    command: 'save_settings',
    args: { settings },
    reply: null,
    resolves: undefined,
  },
  ankiExport: {
    run: (a) => a.ankiExport(),
    command: 'anki_export',
    reply: fx.report(),
  },
  ankiImport: {
    run: (a) => a.ankiImport('Chinese::HSK'),
    command: 'anki_import',
    args: { deck: 'Chinese::HSK' },
    reply: fx.report({ skipped: 2 }),
  },
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('TauriAdapter', () => {
  it('maps every method of the PlatformAdapter (and nothing else) to a command', () => {
    const methods = Object.getOwnPropertyNames(TauriAdapter.prototype).filter(
      (name) => name !== 'constructor',
    )
    expect(methods.sort()).toEqual(Object.keys(CALLS).sort())
  })

  it.each(Object.entries(CALLS))(
    '%s invokes its command with the camelCase arguments',
    async (_name, call) => {
      invokeMock.mockResolvedValueOnce(call.reply)
      const result = await call.run(new TauriAdapter())
      expect(invokeMock).toHaveBeenCalledTimes(1)
      const expected = call.args === undefined ? [call.command] : [call.command, call.args]
      expect(invokeMock.mock.calls[0]).toEqual(expected)
      expect(result).toEqual('resolves' in call ? call.resolves : call.reply)
    },
  )

  it('reports a desktop platform with TTS and Anki', () => {
    expect(new TauriAdapter().capabilities).toEqual({ kind: 'tauri', tts: true, anki: true })
  })

  it('rejects with the command’s error string', async () => {
    invokeMock.mockRejectedValueOnce('database is locked')
    await expect(new TauriAdapter().getStats()).rejects.toBe('database is locked')
  })

  it('turns any other rejection into a readable string', async () => {
    invokeMock.mockRejectedValueOnce(new Error('IPC channel closed'))
    await expect(new TauriAdapter().getDailyQueue()).rejects.toBe('IPC channel closed')
    invokeMock.mockRejectedValueOnce({ kind: 'not_found', message: 'card 9 not found' })
    await expect(new TauriAdapter().getCard(9)).rejects.toBe('card 9 not found')
  })
})
