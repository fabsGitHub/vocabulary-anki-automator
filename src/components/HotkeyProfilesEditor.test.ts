import { mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { CONFIG_STORE_KEY, useConfig, type ConfigStore } from '../composables/useConfig'
import { i18n, setLocale } from '../i18n'
import HotkeyProfilesEditor from './HotkeyProfilesEditor.vue'

function mountEditor(store: ConfigStore, decks: string[] = []): VueWrapper {
  return mount(HotkeyProfilesEditor, {
    props: { decks },
    global: { plugins: [i18n], provide: { [CONFIG_STORE_KEY as symbol]: store } },
  })
}

const profiles = (wrapper: VueWrapper) => wrapper.findAll('[data-test="profile"]')

beforeEach(() => setLocale('en'))

describe('HotkeyProfilesEditor', () => {
  it('adds a profile for the default pair with a unique id and asks for its shortcut', async () => {
    const store = useConfig({ isMac: false })
    const wrapper = mountEditor(store)
    expect(wrapper.find('[data-test="no-profiles"]').exists()).toBe(true)

    await wrapper.find('[data-test="add-profile"]').trigger('click')
    await wrapper.find('[data-test="add-profile"]').trigger('click')
    expect(store.config.value.hotkey_profiles).toEqual([
      {
        id: 'en-de',
        label: 'English → German',
        shortcut: '',
        source_lang: 'en',
        target_lang: 'de',
        deck_name: null,
      },
      {
        id: 'en-de-2',
        label: 'English → German',
        shortcut: '',
        source_lang: 'en',
        target_lang: 'de',
        deck_name: null,
      },
    ])
    expect(profiles(wrapper)).toHaveLength(2)
    expect(profiles(wrapper)[0].find('[data-test="profile-shortcut-error"]').text()).toBe(
      'Required.',
    )
  })

  it('edits every field of a profile', async () => {
    const store = useConfig({ isMac: false })
    const wrapper = mountEditor(store, ['Chinese::HSK'])
    await wrapper.find('[data-test="add-profile"]').trigger('click')
    const profile = profiles(wrapper)[0]
    await profile.find('[data-test="profile-label"]').setValue('Chinese → German')
    await profile.find('[data-test="profile-id"]').setValue('zh-de')
    await profile.find('[data-test="profile-shortcut"]').setValue('CommandOrControl+Shift+H')
    await profile.find('select[id="profile-0-source"]').setValue('zh-CN')
    await profile.find('select[id="profile-0-target"]').setValue('de')
    await profile.find('[data-test="profile-deck"]').setValue('Chinese::HSK')

    expect(store.config.value.hotkey_profiles[0]).toEqual({
      id: 'zh-de',
      label: 'Chinese → German',
      shortcut: 'CommandOrControl+Shift+H',
      source_lang: 'zh-CN',
      target_lang: 'de',
      deck_name: 'Chinese::HSK',
    })
    expect(store.issues.value).toEqual([])
    expect(wrapper.findAll('#profile-decks option').map((o) => o.attributes('value'))).toEqual([
      'Chinese::HSK',
    ])

    await profile.find('[data-test="profile-deck"]').setValue('  ')
    expect(store.config.value.hotkey_profiles[0].deck_name).toBeNull()
  })

  it('removes a profile', async () => {
    const store = useConfig({ isMac: false })
    const wrapper = mountEditor(store)
    await wrapper.find('[data-test="add-profile"]').trigger('click')
    await wrapper.find('[data-test="add-profile"]').trigger('click')
    await profiles(wrapper)[0].find('[data-test="remove-profile"]').trigger('click')
    expect(store.config.value.hotkey_profiles.map((p) => p.id)).toEqual(['en-de-2'])
    expect(profiles(wrapper)).toHaveLength(1)
  })

  it('points at a shortcut another hotkey already uses', async () => {
    const store = useConfig({ isMac: false })
    const wrapper = mountEditor(store)
    await wrapper.find('[data-test="add-profile"]').trigger('click')
    await profiles(wrapper)[0].find('[data-test="profile-shortcut"]').setValue('Ctrl+Shift+C')
    expect(profiles(wrapper)[0].find('[data-test="profile-shortcut-error"]').text()).toBe(
      'Already used by the default hotkey.',
    )
    await profiles(wrapper)[0].find('[data-test="profile-shortcut"]').setValue('Ctrl+Shift+')
    expect(profiles(wrapper)[0].find('[data-test="profile-shortcut-error"]').text()).toContain(
      'Not a valid shortcut',
    )
  })
})
