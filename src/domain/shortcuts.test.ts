import { describe, expect, it } from 'vitest'

import { leavesTypingAlone, parseShortcut, shortcutId } from './shortcuts'

const id = (text: string, isMac: boolean) => {
  const parsed = parseShortcut(text, isMac)
  if (!parsed) throw new Error(`${text} did not parse`)
  return shortcutId(parsed)
}

describe('parseShortcut (global-hotkey grammar)', () => {
  it.each([
    ['CommandOrControl+Shift+C', true, ['shift', 'super'], 'KeyC'],
    ['CommandOrControl+Shift+C', false, ['shift', 'control'], 'KeyC'],
    ['ctrl+alt+KeyX', false, ['control', 'alt'], 'KeyX'],
    ['Option+Space', true, ['alt'], 'Space'],
    ['Super+`', false, ['super'], 'Backquote'],
    ['CmdOrCtrl+Shift+1', true, ['shift', 'super'], 'Digit1'],
    ['Alt+Num5', false, ['alt'], 'Numpad5'],
    ['Shift+Up', false, ['shift'], 'ArrowUp'],
    ['Shift + F12', false, ['shift'], 'F12'],
    ['shift+f24', false, ['shift'], 'F24'],
    ['F8', false, [], 'F8'],
    ['KeyX', false, [], 'KeyX'],
    ['Control+MediaTrackPrev', false, ['control'], 'MediaTrackPrevious'],
  ])('parses %j (mac=%s)', (text, isMac, modifiers, key) => {
    expect(parseShortcut(text, isMac)).toEqual({ modifiers, key })
  })

  it.each([
    '',
    'Shift',
    'Ctrl+Shift',
    'Ctrl+C+Shift',
    'Ctrl+Shift+C+A',
    'Ctrl++C',
    '+C',
    'Ctrl+Shift+',
    'Ctrl+Foo',
    'Hyper+C',
    'Ctrl+F25',
    ' F8',
  ])('rejects %j like the backend does', (text) => {
    expect(parseShortcut(text, false)).toBeNull()
    expect(parseShortcut(text, true)).toBeNull()
  })
})

describe('shortcutId', () => {
  it('gives every spelling of one shortcut the same identity', () => {
    expect(id('CommandOrControl+Shift+C', true)).toBe(id('shift+command+KeyC', true))
    expect(id('Alt+Shift+H', false)).toBe(id('SHIFT+OPTION+h', false))
  })

  it('resolves CommandOrControl to Command on macOS and Control elsewhere', () => {
    expect(id('CmdOrCtrl+C', true)).toBe(id('Cmd+C', true))
    expect(id('CmdOrCtrl+C', true)).not.toBe(id('Ctrl+C', true))
    expect(id('CmdOrCtrl+C', false)).toBe(id('Ctrl+C', false))
    expect(id('CmdOrCtrl+C', false)).not.toBe(id('Super+C', false))
  })

  it('tells different keys and modifier sets apart', () => {
    expect(id('Ctrl+Shift+C', false)).not.toBe(id('Ctrl+Shift+V', false))
    expect(id('Ctrl+Shift+C', false)).not.toBe(id('Ctrl+C', false))
  })
})

describe('leavesTypingAlone (the backend refuses the rest)', () => {
  const safe = (text: string, isMac = false) => {
    const parsed = parseShortcut(text, isMac)
    if (!parsed) throw new Error(`${text} did not parse`)
    return leavesTypingAlone(parsed)
  }

  it.each(['C', 'Shift+C', 'shift+KeyQ', 'Digit1', 'Space', 'Shift+Enter', 'MediaPlayPause'])(
    'refuses %j: it needs Ctrl, Alt or Cmd',
    (text) => expect(safe(text)).toBe(false),
  )

  it.each([
    'Ctrl+C',
    'Alt+C',
    'Super+C',
    'CommandOrControl+Shift+C',
    'F1',
    'F8',
    'Shift+F12',
    'F24',
  ])('accepts %j', (text) => {
    expect(safe(text, false)).toBe(true)
    expect(safe(text, true)).toBe(true)
  })
})
