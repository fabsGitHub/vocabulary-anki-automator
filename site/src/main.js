// Progressive enhancement for the product page. Everything here is optional:
// without JavaScript the page shows the same content, the download links work
// and the menu opens. No network requests, no storage, no cookies.

/** The visitor's platform, as one word the stylesheet can match on. */
function detectOs() {
  const nav = window.navigator
  const platform = (nav.userAgentData && nav.userAgentData.platform) || nav.platform || ''
  const agent = nav.userAgent || ''
  const probe = `${platform} ${agent}`
  if (/android/i.test(probe)) return 'mobile'
  if (/iphone|ipad|ipod/i.test(probe)) return 'mobile'
  // iPadOS reports itself as a Mac; a touch screen gives it away.
  if (/mac/i.test(probe)) return nav.maxTouchPoints > 1 ? 'mobile' : 'mac'
  if (/win/i.test(probe)) return 'windows'
  if (/linux|x11|cros/i.test(probe)) return 'linux'
  return 'other'
}

/** Marks the visitor's platform: the stylesheet highlights the matching download. */
function markOs() {
  document.documentElement.dataset.os = detectOs()
}

/** Shows the copy buttons and copies the referenced element's text on click. */
function setUpCopyButtons() {
  if (!navigator.clipboard || typeof navigator.clipboard.writeText !== 'function') return
  for (const button of document.querySelectorAll('[data-copy]')) {
    const target = document.getElementById(button.dataset.copy)
    if (!target) continue
    const text = button.querySelector('[data-copy-text]')
    const status = button.closest('.checksum')?.querySelector('[data-copy-status]')
    let timer = 0
    button.hidden = false
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(target.textContent.trim())
      } catch {
        return
      }
      button.classList.add('is-copied')
      if (text) text.textContent = button.dataset.copiedLabel
      if (status) status.textContent = button.dataset.copiedLabel
      window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        button.classList.remove('is-copied')
        if (text) text.textContent = button.dataset.label
        if (status) status.textContent = ''
      }, 2000)
    })
  }
}

/** Closes the mobile menu after a link in it is used, on Escape and on a click outside. */
function setUpMenu() {
  const menu = document.querySelector('[data-menu]')
  if (!menu) return
  const close = () => {
    menu.open = false
  }
  menu.addEventListener('click', (event) => {
    if (event.target instanceof Element && event.target.closest('a')) close()
  })
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && menu.open) {
      close()
      menu.querySelector('summary')?.focus()
    }
  })
  document.addEventListener('click', (event) => {
    if (menu.open && event.target instanceof Node && !menu.contains(event.target)) close()
  })
}

/**
 * After the hero's download starts, brings the install steps into view: the
 * preview build needs them on first open.
 */
function setUpDownloadHint() {
  const steps = document.getElementById('install')
  if (!steps) return
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  for (const link of document.querySelectorAll('[data-download]')) {
    link.addEventListener('click', () => {
      window.setTimeout(() => {
        steps.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'start' })
      }, 400)
    })
  }
}

markOs()
setUpCopyButtons()
setUpMenu()
setUpDownloadHint()
