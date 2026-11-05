import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { fakePlatform, mountWith, type FakePlatform } from '../../test/fakePlatform'
import * as fx from '../../test/fixtures'
import ListeningSentence from './ListeningSentence.vue'

let wrapper: VueWrapper | null = null

async function mountExercise(platform: FakePlatform): Promise<VueWrapper> {
  wrapper = mountWith(
    ListeningSentence,
    { platform },
    { props: { exercise: fx.listeningSentence() } },
  )
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)

async function answer(...ids: number[]): Promise<void> {
  for (const id of ids) await find(`[data-test="tile-${id}"]`).trigger('click')
  await find('[data-test="check"]').trigger('click')
  await flushPromises()
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('listening sentence', () => {
  it('synthesizes the sentence and keeps its text hidden until answered', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    expect(platform.synthesizeAudio).toHaveBeenCalledWith('我在学习中文。', 'zh-CN')
    expect(platform.getCardAudio).not.toHaveBeenCalled()
    expect(wrapper!.text()).toContain('Listen and build the sentence you hear.')
    expect(wrapper!.text()).not.toContain('我在学习中文。')
  })

  it('records a correct order', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await answer(1, 3, 0, 2)
    expect(platform.checkExercise).toHaveBeenCalledWith(fx.listeningSentence(), {
      kind: 'order',
      tile_ids: [1, 3, 0, 2],
    })
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'listening_sentence', true)
    expect(find('[data-test="verdict"]').text()).toContain('Correct!')
    expect(find('[data-test="sentence"]').text()).toBe('Sentence: 我在学习中文。')
  })

  it('records a wrong order', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await answer(1, 0)
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'listening_sentence', false)
    expect(find('[data-test="verdict"]').text()).toContain('Not quite.')
    expect(find('[data-test="sentence"]').text()).toBe('Sentence: 我在学习中文。')
    expect(wrapper!.emitted('answered')).toEqual([[false]])
  })

  it('says so when it cannot play the sentence', async () => {
    const platform = fakePlatform()
    platform.capabilities.tts = false
    await mountExercise(platform)
    expect(platform.synthesizeAudio).not.toHaveBeenCalled()
    expect(find('[data-test="audio-status"]').text()).toBe('No audio is available for this.')
  })
})
