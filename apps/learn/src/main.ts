import { createApp } from 'vue'

import App from './App.vue'
import { learnContext, randomSeeds, systemClock } from './context'
import { errorMessage } from './domain/errors'
import { i18n, initialUiLang, setLocale } from './i18n'
import { getPlatform } from './platform'
import './styles.css'

setLocale(initialUiLang())

// The adapter is resolved before mounting: the web build opens its database
// first. A platform that cannot start leaves a readable message, not a blank window.
getPlatform().then(
  (platform) => {
    createApp(App)
      .use(i18n)
      .use(learnContext({ platform, clock: systemClock, seeds: randomSeeds }))
      .mount('#app')
  },
  (err: unknown) => {
    const root = document.getElementById('app')
    if (!root) return
    const message = document.createElement('p')
    message.className = 'notice error startup-error'
    message.setAttribute('role', 'alert')
    message.textContent = i18n.global.t('app.startFailed', { error: errorMessage(err) })
    root.replaceChildren(message)
  },
)
