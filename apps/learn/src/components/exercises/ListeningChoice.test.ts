import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { ListeningChoiceExercise } from '../../platform/types'
import { fakePlatform, mountWith, type FakePlatform } from '../../test/fakePlatform'
import * as fx from '../../test/fixtures'
import ListeningChoice from './ListeningChoice.vue'

let wrapper: VueWrapper | null = null

async function mountExercise(
  platform: FakePlatform,
  exercise: ListeningChoiceExercise = fx.listeningChoice(),
): Promise<VueWrapper> {
  wrapper = mountWith(ListeningChoice, { platform }, { props: { exercise } })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const played = () =>
  (HTMLMediaElement.prototype.play as unknown as { mock: { contexts: HTMLAudioElement[] } }).mock
    .contexts

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('listening choice', () => {
  it('plays the card’s stored recording as soon as it appears', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    await mountExercise(platform)
    expect(platform.getCardAudio).toHaveBeenCalledWith(1)
    expect(platform.synthesizeAudio).not.toHaveBeenCalled()
    expect(played().map((audio) => audio.src)).toEqual(['data:audio/mpeg;base64,SUQz'])
    expect(wrapper!.text()).toContain('Listen and pick the word you hear.')
  })

  it('synthesizes the text when the exercise names no recording', async () => {
    const platform = fakePlatform()
    await mountExercise(
      platform,
      fx.listeningChoice({ audio: { audio_card_id: null, text: '学习', lang: 'zh-CN' } }),
    )
    expect(platform.getCardAudio).not.toHaveBeenCalled()
    expect(platform.synthesizeAudio).toHaveBeenCalledWith('学习', 'zh-CN')
    expect(played().map((audio) => audio.src)).toEqual(['data:audio/mpeg;base64,U1lOVEg='])
  })

  it('plays again on request without fetching again', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    await mountExercise(platform)
    await find('[data-test="play-audio"]').trigger('click')
    await flushPromises()
    expect(platform.getCardAudio).toHaveBeenCalledTimes(1)
    expect(played()).toHaveLength(2)
  })

  it('stays quiet about a refused autoplay but reports a failed fetch', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    const play = HTMLMediaElement.prototype.play as unknown as {
      mockRejectedValueOnce(error: unknown): void
    }
    play.mockRejectedValueOnce(new DOMException('needs a click', 'NotAllowedError'))
    await mountExercise(platform)
    expect(find('[data-test="audio-status"]').text()).toBe('')

    wrapper!.unmount()
    const failing = fakePlatform()
    failing.getCardAudio.mockRejectedValue('Google TTS is unreachable')
    await mountExercise(failing)
    expect(find('[data-test="audio-status"]').text()).toContain('Google TTS is unreachable')
  })

  it('records a correct pick', async () => {
    const platform = fakePlatform()
    const exercise = fx.listeningChoice()
    await mountExercise(platform, exercise)
    await find('[data-test="option-0"]').trigger('click')
    await flushPromises()
    expect(platform.checkExercise).toHaveBeenCalledWith(exercise, { kind: 'choice', index: 0 })
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'listening_choice', true)
    expect(find('[data-test="verdict"]').text()).toContain('Correct!')
    expect(wrapper!.emitted('answered')).toEqual([[true]])
  })

  it('records a wrong pick and names the word that was played', async () => {
    const platform = fakePlatform()
    await mountExercise(platform)
    await find('[data-test="option-2"]').trigger('click')
    await flushPromises()
    expect(platform.recordExerciseResult).toHaveBeenCalledWith(1, 'listening_choice', false)
    expect(find('[data-test="solution"]').text()).toBe('Answer: 学习')
    expect(wrapper!.emitted('answered')).toEqual([[false]])
  })
})
