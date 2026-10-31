import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '../i18n'
import ConsentDialog from './ConsentDialog.vue'

function mountDialog(action: 'anki_install' | 'ankiconnect_provision', ankiVersion = '24.06.3') {
  return mount(ConsentDialog, { props: { action, ankiVersion }, global: { plugins: [i18n] } })
}

beforeEach(() => setLocale('en'))

describe('ConsentDialog', () => {
  it('states what AnkiConnect provisioning downloads, from where, how it is verified and where it goes', () => {
    const text = mountDialog('ankiconnect_provision').text()
    expect(text).toContain('Install the AnkiConnect add-on?')
    expect(text).toContain('__init__.py, edit.py, util.py and web.py')
    expect(text).toContain(
      'github.com/FooSoft/anki-connect/tree/4064fa142785975255457abd6a496015f5b71f38/plugin',
    )
    expect(text).toContain('SHA-256')
    expect(text).toContain('before anything is written')
    expect(text).toContain('addons21/2055492159')
    expect(text).toContain('127.0.0.1:8765')
    expect(text).toContain('2185b1243b924152cbac7a7668b40c0bf713902619bfc8ede5c42465066d0f05')
  })

  it('describes the AnkiConnect config as exactly as § 8.4 does, never as app-exclusive', () => {
    // The config binds loopback and allows no web origin (an empty
    // webCorsOriginList: both apps call AnkiConnect from Rust, without an
    // Origin). Any local program can still connect, so "only this app" would
    // be a promise the consent dialog cannot keep, and no web page is let in,
    // this app's windows included.
    for (const locale of ['en', 'de'] as const) {
      setLocale(locale)
      const text = mountDialog('ankiconnect_provision').text()
      expect(text, locale).toContain('127.0.0.1:8765')
      expect(text, locale).toMatch(
        locale === 'en' ? /refuse requests from all web pages/ : /Anfragen von allen Webseiten ab/,
      )
      expect(text, locale).not.toMatch(/other than this app|außer dieser App/i)
      expect(text, locale).not.toMatch(/only accepts connections from this app/i)
      expect(text, locale).not.toMatch(/nur Verbindungen dieser App/i)
    }
  })

  it('states what the Anki install downloads, from where, how it is verified and where it goes', () => {
    const text = mountDialog('anki_install').text()
    expect(text).toContain('Install Anki 24.06.3?')
    expect(text).toContain('github.com/ankitects/anki/releases/tag/24.06.3')
    expect(text).toContain('SHA-256')
    expect(text).toContain('nothing is installed or run')
    expect(text).toContain('/Applications')
    expect(text).toContain('Program Files')
    expect(text).toContain('anki-24.06.3-mac-apple-qt6.dmg')
    expect(text).toContain('ce2f728c98ab3711450fbc066b8fbe27e983684e5d6195be5f451475d1a48d35')
  })

  it('lists no checksums for a release they do not belong to', () => {
    const text = mountDialog('anki_install', '25.02').text()
    expect(text).toContain('Install Anki 25.02?')
    expect(text).not.toContain('anki-24.06.3-mac-apple-qt6.dmg')
  })

  it('emits agree only on the agree button', async () => {
    const wrapper = mountDialog('ankiconnect_provision')
    await wrapper.find('[data-test="consent-agree"]').trigger('click')
    expect(wrapper.emitted('agree')).toHaveLength(1)
    expect(wrapper.emitted('cancel')).toBeUndefined()
  })

  it('cancels on the cancel button, Escape and a click outside, never agreeing', async () => {
    const wrapper = mountDialog('anki_install')
    await wrapper.find('[data-test="consent-cancel"]').trigger('click')
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Escape' })
    await wrapper.find('.modal-overlay').trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(3)
    expect(wrapper.emitted('agree')).toBeUndefined()
  })

  it('is shown in German too', () => {
    setLocale('de')
    expect(mountDialog('ankiconnect_provision').text()).toContain('Zustimmen und installieren')
  })
})
