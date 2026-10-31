import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { Exercise, ExerciseKind } from '../platform/types'
import {
  countingSeeds,
  deferred,
  fakePlatform,
  mountWith,
  type FakePlatform,
} from '../test/fakePlatform'
import * as fx from '../test/fixtures'
import PracticeView from './PracticeView.vue'

const LEARNED = [
  fx.learnedCard(),
  fx.learnedCard({
    card_id: 2,
    front: 'serendipity',
    back: 'glücklicher Zufall',
    pinyin: null,
    source_lang: 'en',
    target_lang: 'de',
    script_kind: 'none',
    has_audio: true,
  }),
]

const EXERCISES: Record<ExerciseKind, Exercise> = {
  multiple_choice: fx.multipleChoice(),
  sentence_builder: fx.sentenceBuilder(),
  listening_choice: fx.listeningChoice(),
  listening_sentence: fx.listeningSentence(),
}

const COMPONENTS: Record<ExerciseKind, string> = {
  multiple_choice: 'multiple-choice',
  sentence_builder: 'sentence-builder',
  listening_choice: 'listening-choice',
  listening_sentence: 'listening-sentence',
}

let wrapper: VueWrapper | null = null

function practicePlatform(tts = true): FakePlatform {
  const platform = fakePlatform({ tts })
  platform.listLearnedCards.mockResolvedValue(LEARNED)
  platform.nextExercise.mockImplementation(async (_cardId, kind) => EXERCISES[kind])
  return platform
}

async function mountPractice(platform: FakePlatform): Promise<VueWrapper> {
  wrapper = mountWith(PracticeView, { platform, seeds: countingSeeds() })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)

async function generate(): Promise<void> {
  await find('[data-test="generate"]').trigger('click')
  await flushPromises()
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('practice', () => {
  it('offers the learned words, and only those', async () => {
    const platform = practicePlatform()
    await mountPractice(platform)
    expect(platform.listLearnedCards).toHaveBeenCalledTimes(1)
    const options = wrapper!.findAll('[data-test="practice-card"] option')
    expect(options.map((option) => option.text())).toEqual([
      '学习 · Chinese → English',
      'serendipity · English → German',
    ])
    // The list never gives the answers away.
    expect(find('[data-test="practice-card"]').text()).not.toContain('to study')
    expect(find('[data-test="practice-card"]').text()).not.toContain('xué')
  })

  it('asks for an exercise of the chosen word and type, with a fresh seed each time', async () => {
    const platform = practicePlatform()
    await mountPractice(platform)
    await find('[data-test="practice-card"]').setValue('2')
    await find('[data-test="kind-sentence_builder"]').setValue(true)
    await generate()
    expect(platform.nextExercise).toHaveBeenLastCalledWith(2, 'sentence_builder', 1)
    expect(find('[data-test="sentence-builder"]').exists()).toBe(true)

    await generate()
    expect(platform.nextExercise).toHaveBeenLastCalledWith(2, 'sentence_builder', 2)
  })

  it.each(Object.keys(EXERCISES) as ExerciseKind[])('renders a %s exercise', async (kind) => {
    const platform = practicePlatform()
    await mountPractice(platform)
    await find(`[data-test="kind-${kind}"]`).setValue(true)
    await generate()
    expect(platform.nextExercise).toHaveBeenCalledWith(1, kind, 1)
    const shown = Object.values(COMPONENTS).filter((name) => find(`[data-test="${name}"]`).exists())
    expect(shown).toEqual([COMPONENTS[kind]])
  })

  it('moves on to a new, unanswered exercise after an answer', async () => {
    const platform = practicePlatform()
    await mountPractice(platform)
    const button = () => find('[data-test="generate"]')
    expect(button().classes()).toContain('btn-primary')
    await generate()
    // While the exercise waits for its answer, skipping it is the lesser action.
    expect(button().text()).toBe('Skip')
    expect(button().classes()).toContain('btn-secondary')
    await find('[data-test="option-1"]').trigger('click')
    await flushPromises()
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'multiple_choice', true)
    expect(find('[data-test="verdict"]').exists()).toBe(true)
    expect(button().text()).toBe('Next exercise')
    expect(button().classes()).toContain('btn-primary')

    await generate()
    expect(platform.nextExercise).toHaveBeenCalledTimes(2)
    expect(find('[data-test="verdict"]').exists()).toBe(false)
    expect(find('[data-test="option-1"]').attributes('disabled')).toBeUndefined()
  })

  it('drops an exercise that arrives after the word or the type changed', async () => {
    const platform = practicePlatform()
    const late = deferred<Exercise>()
    platform.nextExercise.mockReturnValueOnce(late.promise)
    await mountPractice(platform)
    await generate()
    expect(find('[data-test="generate"]').text()).toBe('Preparing the exercise…')

    await find('[data-test="practice-card"]').setValue('2')
    late.resolve(EXERCISES.multiple_choice)
    await flushPromises()
    expect(find('[data-test="exercise"]').exists()).toBe(false)
    expect(find('[data-test="generate"]').text()).toBe('Start exercise')

    // A change of type drops it as well, and the next request is the new choice's.
    const stale = deferred<Exercise>()
    platform.nextExercise.mockReturnValueOnce(stale.promise)
    await generate()
    await find('[data-test="kind-sentence_builder"]').setValue(true)
    stale.reject('no learned example sentence uses card 2')
    await flushPromises()
    expect(find('[data-test="generate-error"]').exists()).toBe(false)
    await generate()
    expect(platform.nextExercise).toHaveBeenLastCalledWith(2, 'sentence_builder', 3)
    expect(find('[data-test="sentence-builder"]').exists()).toBe(true)
  })

  it('keeps keyboard focus on the generate button, and returns it there after an answer', async () => {
    const platform = practicePlatform()
    const pending = deferred<Exercise>()
    platform.nextExercise.mockReturnValueOnce(pending.promise)
    wrapper = mountWith(PracticeView, { platform }, { attachTo: document.body })
    await flushPromises()
    const button = find('[data-test="generate"]')
    ;(button.element as HTMLElement).focus()
    await generate()
    // Not disabled while busy, so it keeps its focus (and a second press does nothing).
    expect(button.attributes('disabled')).toBeUndefined()
    expect(button.attributes('aria-disabled')).toBe('true')
    await generate()
    expect(platform.nextExercise).toHaveBeenCalledTimes(1)
    pending.resolve(EXERCISES.multiple_choice)
    await flushPromises()
    expect(button.attributes('aria-disabled')).toBeUndefined()
    expect(document.activeElement).toBe(button.element)

    // Answering from the keyboard disables the options, which would drop focus to the page.
    const option = find('[data-test="option-1"]')
    ;(option.element as HTMLElement).focus()
    await option.trigger('click')
    await flushPromises()
    expect(find('[data-test="verdict"]').exists()).toBe(true)
    expect(document.activeElement).toBe(button.element)
    expect(button.text()).toBe('Next exercise')
  })

  it('says why there is no exercise of that type for the word', async () => {
    const platform = practicePlatform()
    platform.nextExercise.mockRejectedValueOnce('no learned example sentence uses card 1')
    await mountPractice(platform)
    await find('[data-test="kind-sentence_builder"]').setValue(true)
    await generate()
    expect(find('[data-test="generate-error"]').text()).toContain(
      'no learned example sentence uses card 1',
    )
    expect(find('[data-test="exercise"]').exists()).toBe(false)
  })

  it('offers listening only when it can play the sound', async () => {
    const platform = practicePlatform(false)
    await mountPractice(platform)
    const kind = (name: string) => find(`[data-test="kind-${name}"]`)
    expect(kind('multiple_choice').attributes('disabled')).toBeUndefined()
    expect(kind('sentence_builder').attributes('disabled')).toBeUndefined()
    expect(kind('listening_sentence').attributes('disabled')).toBeDefined()
    // 学习 has no recording.
    expect(kind('listening_choice').attributes('disabled')).toBeDefined()

    // serendipity has one.
    await find('[data-test="practice-card"]').setValue('2')
    expect(kind('listening_choice').attributes('disabled')).toBeUndefined()
    expect(kind('listening_sentence').attributes('disabled')).toBeDefined()
  })

  it('says what to do when nothing is learned yet', async () => {
    const platform = fakePlatform()
    await mountPractice(platform)
    expect(find('[data-test="practice-empty"]').text()).toContain(
      'Learn a few words in Review first',
    )
    expect(find('[data-test="generate"]').exists()).toBe(false)
  })

  it('shows why the words could not be loaded', async () => {
    const platform = fakePlatform()
    platform.listLearnedCards.mockRejectedValueOnce('database is locked')
    await mountPractice(platform)
    expect(find('[data-test="practice-error"]').text()).toContain('database is locked')
  })
})
