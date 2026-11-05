import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { AudioSource } from '../composables/useAudio'
import { deferred, fakePlatform, mountWith, type FakePlatform } from '../test/fakePlatform'
import AudioButton from './AudioButton.vue'

const WORD: AudioSource = { cardId: 1, text: '学习', lang: 'zh-CN' }
const SENTENCE: AudioSource = { cardId: null, text: '我在学习中文。', lang: 'zh-CN' }

type MediaSpy = { mock: { contexts: HTMLMediaElement[] } }
const played = () => (HTMLMediaElement.prototype.play as unknown as MediaSpy).mock.contexts
const paused = () => (HTMLMediaElement.prototype.pause as unknown as MediaSpy).mock.contexts

let mounted: VueWrapper[] = []

async function mountButton(platform: FakePlatform, source: AudioSource): Promise<VueWrapper> {
  const wrapper = mountWith(
    AudioButton,
    { platform },
    { props: { source, label: 'Play audio', autoplay: true } },
  )
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

function unmount(wrapper: VueWrapper): void {
  mounted = mounted.filter((other) => other !== wrapper)
  wrapper.unmount()
}

afterEach(() => {
  for (const wrapper of mounted) wrapper.unmount()
  mounted = []
})

describe('the play button', () => {
  it('fetches once and plays the same recording again on request', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    const button = await mountButton(platform, WORD)
    await button.find('[data-test="play-audio"]').trigger('click')
    await flushPromises()
    expect(platform.getCardAudio).toHaveBeenCalledTimes(1)
    expect(played().map((player) => (player as HTMLAudioElement).src)).toEqual([
      'data:audio/mpeg;base64,SUQz',
      'data:audio/mpeg;base64,SUQz',
    ])
  })

  it('keeps the focus of a keyboard user while the recording loads, and loads it once', async () => {
    const platform = fakePlatform()
    const pending = deferred<string | null>()
    platform.getCardAudio.mockReturnValueOnce(pending.promise)
    const wrapper = mountWith(
      AudioButton,
      { platform },
      { props: { source: WORD, label: 'Play audio' }, attachTo: document.body },
    )
    mounted.push(wrapper)
    const button = wrapper.find('[data-test="play-audio"]')
    ;(button.element as HTMLElement).focus()
    await button.trigger('click')
    expect(button.attributes('aria-disabled')).toBe('true')
    expect(button.attributes('disabled')).toBeUndefined()
    expect(document.activeElement).toBe(button.element)
    await button.trigger('click')
    expect(platform.getCardAudio).toHaveBeenCalledTimes(1)

    pending.resolve('SUQz')
    await flushPromises()
    expect(button.attributes('aria-disabled')).toBeUndefined()
    expect(played()).toHaveLength(1)
  })

  it('stops its recording when it goes away', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    const button = await mountButton(platform, WORD)
    const [player] = played()
    expect(paused()).not.toContain(player)
    unmount(button)
    expect(paused()).toContain(player)
  })

  it('never plays a recording that arrives after it went away', async () => {
    const platform = fakePlatform()
    const pending = deferred<string | null>()
    platform.synthesizeAudio.mockReturnValueOnce(pending.promise)
    const button = await mountButton(platform, SENTENCE)
    expect(platform.synthesizeAudio).toHaveBeenCalledWith('我在学习中文。', 'zh-CN')
    unmount(button)
    pending.resolve('U1lOVEg=')
    await flushPromises()
    expect(played()).toHaveLength(0)
  })

  it('leaves alone a recording another button started since', async () => {
    const platform = fakePlatform()
    platform.getCardAudio.mockResolvedValue('SUQz')
    const first = await mountButton(platform, WORD)
    const second = await mountButton(platform, { ...WORD, cardId: 2 })
    const [firstPlayer, secondPlayer] = played()
    // Starting the second recording stopped the first.
    expect(paused()).toContain(firstPlayer)

    unmount(first)
    expect(paused()).not.toContain(secondPlayer)
    unmount(second)
    expect(paused()).toContain(secondPlayer)
  })
})
