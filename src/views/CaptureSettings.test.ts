import { mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { CONFIG_STORE_KEY, useConfig, type ConfigStore } from '../composables/useConfig'
import { i18n, setLocale } from '../i18n'
import { resetTauri } from '../test/fakeTauri'
import CaptureSettings from './CaptureSettings.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

function mountTab(store: ConfigStore): VueWrapper {
  return mount(CaptureSettings, {
    global: { plugins: [i18n], provide: { [CONFIG_STORE_KEY as symbol]: store } },
  })
}

beforeEach(() => {
  resetTauri()
  setLocale('en')
})

describe('CaptureSettings', () => {
  it('calls the source language a fallback only while languages are listed under "My languages"', async () => {
    const store = useConfig({ isMac: false })
    const wrapper = mountTab(store)
    expect(wrapper.find('[data-test="source-fallback-hint"]').exists()).toBe(false)

    store.config.value.learning.languages.push({
      lang: 'fr',
      deck_name: 'french',
      model_name: 'AppleStyleFinalV3',
      field_mapping: null,
      create_reverse: true,
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="source-fallback-hint"]').text()).toBe(
      'Your default hotkey recognizes the languages listed under “My languages”. This source language is used only for text that matches none of them.',
    )

    // A problem with the source language itself takes the hint's place.
    await wrapper.find('#source-lang').setValue('fr')
    expect(wrapper.find('[data-test="source-fallback-hint"]').exists()).toBe(true)
    store.config.value.translation.source_lang = 'xx'
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="source-fallback-hint"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Unsupported language.')

    store.config.value.learning.languages = []
    store.config.value.translation.source_lang = 'en'
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="source-fallback-hint"]').exists()).toBe(false)
  })
})
