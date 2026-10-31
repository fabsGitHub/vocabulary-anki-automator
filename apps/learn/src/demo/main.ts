/**
 * The live demo's entry point (`--mode demo` loads it instead of
 * `../main.ts`, docs/live-demo.md): the same app on the web platform,
 * opened through {@link openDemo}, which prepares the made-up history
 * first, under a root that adds the demo's bar.
 */

import { createApp } from 'vue'

import { learnContext, randomSeeds } from '../context'
import { errorMessage } from '../domain/errors'
import { UI_LANGS, i18n, initialUiLang, setLocale } from '../i18n'
import '../styles.css'
import './demo.css'
import DemoRoot from './DemoRoot.vue'
import { DEMO_KEY, openDemo } from './demo'
import { demoMessages } from './messages'

const lang = setLocale(initialUiLang())
// Unlike the plain web build, the demo has captures, so its daily prompt
// says where they come in.
for (const each of UI_LANGS) {
  i18n.global.mergeLocaleMessage(each, { prompt: { introWeb: demoMessages[each].promptIntro } })
}

const root = document.getElementById('app')
const preparing = document.createElement('p')
preparing.className = 'input-hint spinner startup-notice'
preparing.textContent = demoMessages[lang].preparing
root?.replaceChildren(preparing)

openDemo().then(
  (demo) => {
    createApp(DemoRoot)
      .use(i18n)
      .use(learnContext({ platform: demo.platform, clock: demo.clock, seeds: randomSeeds }))
      .provide(DEMO_KEY, demo.controller)
      .mount('#app')
  },
  (err: unknown) => {
    if (!root) return
    const message = document.createElement('p')
    message.className = 'notice error startup-error'
    message.setAttribute('role', 'alert')
    message.textContent = i18n.global.t('app.startFailed', { error: errorMessage(err) })
    root.replaceChildren(message)
  },
)
