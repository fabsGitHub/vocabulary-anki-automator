import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { MultipleChoiceExercise } from '../../platform/types'
import { fakePlatform, mountWith, pressKey, type FakePlatform } from '../../test/fakePlatform'
import * as fx from '../../test/fixtures'
import MultipleChoice from './MultipleChoice.vue'

let wrapper: VueWrapper | null = null

async function mountExercise(
  platform: FakePlatform,
  exercise: MultipleChoiceExercise = fx.multipleChoice(),
): Promise<VueWrapper> {
  wrapper = mountWith(
    MultipleChoice,
    { platform },
    { props: { exercise }, attachTo: document.body },
  )
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const option = (index: number) => find(`[data-test="option-${index}"]`)

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('multiple choice', () => {
  it('shows the prompt and exactly the options it was given, in order', async () => {
    await mountExercise(fakePlatform())
    expect(wrapper!.text()).toContain('What does this mean?')
    expect(find('[data-test="prompt"]').text()).toBe('学习')
    expect(find('[data-test="prompt"]').attributes('lang')).toBe('zh-CN')
    const texts = wrapper!.findAll('.choice-text').map((node) => node.text())
    expect(texts).toEqual(['teacher', 'to study', 'China', 'to eat'])
    expect(option(0).find('.choice-text').attributes('lang')).toBe('en')
    expect(find('[data-test="feedback"]').text()).toBe('')
  })

  it('asks the question of its direction', async () => {
    await mountExercise(
      fakePlatform(),
      fx.multipleChoice({ direction: 'back_to_front', prompt: 'to study', prompt_lang: 'en' }),
    )
    expect(wrapper!.text()).toContain('Which word means this?')
    wrapper!.unmount()
    await mountExercise(fakePlatform(), fx.multipleChoice({ direction: 'front_to_pinyin' }))
    expect(wrapper!.text()).toContain('What is the pinyin?')
  })

  it('checks a clicked answer and records a correct result', async () => {
    const platform = fakePlatform()
    const exercise = fx.multipleChoice()
    await mountExercise(platform, exercise)
    await option(1).trigger('click')
    await flushPromises()

    expect(platform.checkExercise).toHaveBeenCalledWith(exercise, { kind: 'choice', index: 1 })
    expect(platform.recordExerciseResult).toHaveBeenCalledTimes(1)
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'multiple_choice', true)
    expect(find('[data-test="verdict"]').text()).toContain('Correct!')
    expect(find('[data-test="xp"]').text()).toBe('+5 XP')
    expect(find('[data-test="solution"]').exists()).toBe(false)
    expect(option(1).attributes('data-state')).toBe('correct')
    expect(wrapper!.emitted('answered')).toEqual([[true]])
  })

  it('takes the answer from its number key and records a wrong result', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    const event = pressKey('3')
    await flushPromises()

    expect(event.defaultPrevented).toBe(true)
    expect(platform.checkExercise).toHaveBeenCalledWith(expect.anything(), {
      kind: 'choice',
      index: 2,
    })
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'multiple_choice', false)
    expect(find('[data-test="verdict"]').text()).toContain('Not quite.')
    expect(find('[data-test="xp"]').exists()).toBe(false)
    expect(find('[data-test="solution"]').text()).toBe('Answer: to study')
    expect(option(2).attributes('data-state')).toBe('wrong')
    expect(option(1).attributes('data-state')).toBe('correct')
    expect(wrapper!.emitted('answered')).toEqual([[false]])
  })

  it('takes one answer only', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await option(0).trigger('click')
    await flushPromises()
    await option(1).trigger('click')
    pressKey('2')
    await flushPromises()
    expect(platform.checkExercise).toHaveBeenCalledTimes(1)
    expect(platform.recordExerciseResult).toHaveBeenCalledTimes(1)
    expect(option(0).attributes('disabled')).toBeDefined()
  })

  it('ignores number keys beyond its options', async () => {
    const platform = fakePlatform()
    await mountExercise(
      platform,
      fx.multipleChoice({ options: fx.multipleChoice().options.slice(0, 2) }),
    )
    pressKey('4')
    await flushPromises()
    expect(platform.checkExercise).not.toHaveBeenCalled()
  })

  it('can be answered again when the check fails', async () => {
    const platform = fakePlatform()
    platform.checkExercise.mockRejectedValueOnce('IPC channel closed')
    await mountExercise(platform)
    await option(1).trigger('click')
    await flushPromises()
    expect(find('[data-test="check-error"]').text()).toContain('IPC channel closed')
    expect(platform.recordExerciseResult).not.toHaveBeenCalled()

    await option(1).trigger('click')
    await flushPromises()
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'multiple_choice', true)
  })

  it('still shows the verdict when the result cannot be recorded', async () => {
    const platform = fakePlatform()
    platform.recordExerciseResult.mockRejectedValueOnce('database is locked')
    await mountExercise(platform)
    await option(1).trigger('click')
    await flushPromises()
    expect(find('[data-test="verdict"]').text()).toContain('Correct!')
    expect(find('[data-test="record-error"]').text()).toContain('database is locked')
  })
})
