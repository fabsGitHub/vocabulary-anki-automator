import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { SentenceBuilderExercise } from '../../platform/types'
import { fakePlatform, mountWith, type FakePlatform } from '../../test/fakePlatform'
import * as fx from '../../test/fixtures'
import SentenceBuilder from './SentenceBuilder.vue'

let wrapper: VueWrapper | null = null

async function mountExercise(
  platform: FakePlatform,
  exercise: SentenceBuilderExercise = fx.sentenceBuilder(),
): Promise<VueWrapper> {
  wrapper = mountWith(SentenceBuilder, { platform }, { props: { exercise } })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const placed = () => wrapper!.findAll('[data-test="tile-answer"] button').map((tile) => tile.text())

async function tap(...ids: number[]): Promise<void> {
  for (const id of ids) await find(`[data-test="tile-${id}"]`).trigger('click')
}

async function check(): Promise<void> {
  await find('[data-test="check"]').trigger('click')
  await flushPromises()
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('sentence builder', () => {
  it('shows the translation and the tiles, but not the sentence it asks for', async () => {
    await mountExercise(fakePlatform())
    expect(find('[data-test="prompt"]').text()).toBe('I am studying Chinese.')
    const bank = wrapper!.findAll('[data-test="tile-bank"] button').map((tile) => tile.text())
    expect(bank).toEqual(['在', '中文', '我', '学习', '老师'])
    expect(find('[data-test="tile-0"]').attributes('lang')).toBe('zh-CN')
    expect(wrapper!.text()).not.toContain('我在学习中文。')
    expect(find('[data-test="check"]').attributes('disabled')).toBeDefined()
  })

  it('asks for the order alone when the capture has no translation', async () => {
    await mountExercise(fakePlatform(), fx.sentenceBuilder({ prompt: '' }))
    expect(find('[data-test="prompt"]').exists()).toBe(false)
    expect(wrapper!.text()).toContain('Put the words in the right order.')
  })

  it('places tapped tiles in order and takes a tapped placed tile back', async () => {
    await mountExercise(fakePlatform())
    await tap(2, 0, 4)
    expect(placed()).toEqual(['我', '在', '老师'])
    expect(find('[data-test="tile-4"]').attributes('disabled')).toBeDefined()

    await find('[data-test="placed-4"]').trigger('click')
    expect(placed()).toEqual(['我', '在'])
    expect(find('[data-test="tile-4"]').attributes('disabled')).toBeUndefined()

    await tap(4)
    expect(placed()).toEqual(['我', '在', '老师'])
  })

  it('checks the placed tile ids and records a correct result', async () => {
    const platform = fakePlatform()
    const exercise = fx.sentenceBuilder()
    await mountExercise(platform, exercise)
    await tap(2, 0, 3, 1)
    await check()

    expect(platform.checkExercise).toHaveBeenCalledWith(exercise, {
      kind: 'order',
      tile_ids: [2, 0, 3, 1],
    })
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'sentence_builder', true)
    expect(find('[data-test="verdict"]').text()).toContain('Correct!')
    expect(find('[data-test="xp"]').text()).toBe('+5 XP')
    expect(find('[data-test="sentence"]').text()).toBe('Sentence: 我在学习中文。')
    expect(wrapper!.emitted('answered')).toEqual([[true]])
  })

  it('records a wrong order as wrong and shows the sentence', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await tap(0, 2, 3, 1)
    await check()

    expect(platform.checkExercise).toHaveBeenCalledWith(expect.anything(), {
      kind: 'order',
      tile_ids: [0, 2, 3, 1],
    })
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'sentence_builder', false)
    expect(find('[data-test="verdict"]').text()).toContain('Not quite.')
    expect(find('[data-test="sentence"]').text()).toBe('Sentence: 我在学习中文。')
    expect(wrapper!.emitted('answered')).toEqual([[false]])
  })

  it('freezes the tiles once answered', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await tap(2, 0, 3, 1)
    await check()
    await find('[data-test="placed-2"]').trigger('click')
    await tap(4)
    expect(placed()).toEqual(['我', '在', '学习', '中文'])
    await check()
    expect(platform.checkExercise).toHaveBeenCalledTimes(1)
  })
})
