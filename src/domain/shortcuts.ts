/**
 * Shortcut parsing, mirrored from `global-hotkey` 0.8 (`HotKey::from_str`),
 * which is what `tauri-plugin-global-shortcut` uses when `apply_settings`
 * registers the configured shortcuts.
 *
 * The backend stays the authority (it rejects what it cannot register), but
 * checking the same grammar here lets the form point at the broken field
 * before anything is sent, and lets "all shortcuts must be distinct" compare
 * shortcuts the way the OS does: `CmdOrCtrl+Shift+C` and `shift+command+KeyC`
 * are the same shortcut on macOS.
 */

export type Modifier = 'shift' | 'control' | 'alt' | 'super'

/** A parsed shortcut: modifiers in a fixed order, and one `Code` name such as `KeyC`. */
export interface Shortcut {
  modifiers: Modifier[]
  key: string
}

const MODIFIER_ORDER: Modifier[] = ['shift', 'control', 'alt', 'super']

/** `CommandOrControl` is Command (`super`) on macOS and Control everywhere else. */
type ModifierToken = Modifier | 'cmd_or_ctrl'

const MODIFIER_TOKENS = new Map<string, ModifierToken>([
  ['OPTION', 'alt'],
  ['ALT', 'alt'],
  ['CONTROL', 'control'],
  ['CTRL', 'control'],
  ['COMMAND', 'super'],
  ['CMD', 'super'],
  ['SUPER', 'super'],
  ['SHIFT', 'shift'],
  ['COMMANDORCONTROL', 'cmd_or_ctrl'],
  ['COMMANDORCTRL', 'cmd_or_ctrl'],
  ['CMDORCTRL', 'cmd_or_ctrl'],
  ['CMDORCONTROL', 'cmd_or_ctrl'],
])

/** `Code` name -> the uppercase spellings `parse_key` accepts for it. */
const NAMED_KEYS: Record<string, string[]> = {
  Backquote: ['BACKQUOTE', '`'],
  Backslash: ['BACKSLASH', '\\'],
  BracketLeft: ['BRACKETLEFT', '['],
  BracketRight: ['BRACKETRIGHT', ']'],
  Pause: ['PAUSE', 'PAUSEBREAK'],
  Comma: ['COMMA', ','],
  Equal: ['EQUAL', '='],
  Minus: ['MINUS', '-'],
  Period: ['PERIOD', '.'],
  Quote: ['QUOTE', "'"],
  Semicolon: ['SEMICOLON', ';'],
  Slash: ['SLASH', '/'],
  Backspace: ['BACKSPACE'],
  CapsLock: ['CAPSLOCK'],
  Enter: ['ENTER'],
  Space: ['SPACE'],
  Tab: ['TAB'],
  Delete: ['DELETE'],
  End: ['END'],
  Home: ['HOME'],
  Insert: ['INSERT'],
  PageDown: ['PAGEDOWN'],
  PageUp: ['PAGEUP'],
  PrintScreen: ['PRINTSCREEN'],
  ScrollLock: ['SCROLLLOCK'],
  ArrowDown: ['ARROWDOWN', 'DOWN'],
  ArrowLeft: ['ARROWLEFT', 'LEFT'],
  ArrowRight: ['ARROWRIGHT', 'RIGHT'],
  ArrowUp: ['ARROWUP', 'UP'],
  NumLock: ['NUMLOCK'],
  NumpadAdd: ['NUMPADADD', 'NUMADD', 'NUMPADPLUS', 'NUMPLUS'],
  NumpadDecimal: ['NUMPADDECIMAL', 'NUMDECIMAL'],
  NumpadDivide: ['NUMPADDIVIDE', 'NUMDIVIDE'],
  NumpadEnter: ['NUMPADENTER', 'NUMENTER'],
  NumpadEqual: ['NUMPADEQUAL', 'NUMEQUAL'],
  NumpadMultiply: ['NUMPADMULTIPLY', 'NUMMULTIPLY'],
  NumpadSubtract: ['NUMPADSUBTRACT', 'NUMSUBTRACT'],
  Escape: ['ESCAPE', 'ESC'],
  AudioVolumeDown: ['AUDIOVOLUMEDOWN', 'VOLUMEDOWN'],
  AudioVolumeUp: ['AUDIOVOLUMEUP', 'VOLUMEUP'],
  AudioVolumeMute: ['AUDIOVOLUMEMUTE', 'VOLUMEMUTE'],
  MediaPlay: ['MEDIAPLAY'],
  MediaPause: ['MEDIAPAUSE'],
  MediaPlayPause: ['MEDIAPLAYPAUSE'],
  MediaStop: ['MEDIASTOP'],
  MediaTrackNext: ['MEDIATRACKNEXT'],
  MediaTrackPrevious: ['MEDIATRACKPREV', 'MEDIATRACKPREVIOUS'],
}

function buildKeyTokens(): Map<string, string> {
  const tokens = new Map<string, string>()
  const add = (code: string, spellings: string[]) => {
    for (const spelling of spellings) tokens.set(spelling, code)
  }
  for (const [code, spellings] of Object.entries(NAMED_KEYS)) add(code, spellings)
  for (let digit = 0; digit <= 9; digit++) {
    add(`Digit${digit}`, [`DIGIT${digit}`, `${digit}`])
    add(`Numpad${digit}`, [`NUMPAD${digit}`, `NUM${digit}`])
  }
  for (let offset = 0; offset < 26; offset++) {
    const letter = String.fromCharCode(65 + offset)
    add(`Key${letter}`, [`KEY${letter}`, letter])
  }
  for (let n = 1; n <= 24; n++) add(`F${n}`, [`F${n}`])
  return tokens
}

const KEY_TOKENS = buildKeyTokens()

/** Whether this webview runs on macOS, where `CommandOrControl` means Command. */
export const IS_MAC: boolean =
  typeof navigator !== 'undefined' && /mac/i.test(navigator.platform || navigator.userAgent)

/**
 * Parses `text` with `global-hotkey`'s grammar: `+`-separated tokens, any
 * modifiers first, exactly one key last, case-insensitive. A lone key needs no
 * modifier (and, as upstream, is not trimmed). `null` when the backend would
 * refuse the shortcut.
 */
export function parseShortcut(text: string, isMac: boolean = IS_MAC): Shortcut | null {
  const tokens = text.split('+')
  if (tokens.length === 1) {
    const key = KEY_TOKENS.get(tokens[0].toUpperCase())
    return key ? { modifiers: [], key } : null
  }

  const modifiers = new Set<Modifier>()
  let key: string | null = null
  for (const raw of tokens) {
    const token = raw.trim()
    if (!token || key !== null) return null
    const upper = token.toUpperCase()
    const modifier = MODIFIER_TOKENS.get(upper)
    if (modifier) {
      modifiers.add(modifier === 'cmd_or_ctrl' ? (isMac ? 'super' : 'control') : modifier)
      continue
    }
    const parsedKey = KEY_TOKENS.get(upper)
    if (!parsedKey) return null
    key = parsedKey
  }
  if (key === null) return null
  return { modifiers: MODIFIER_ORDER.filter((m) => modifiers.has(m)), key }
}

/** A stable identity: two spellings of one shortcut give the same id. */
export function shortcutId(shortcut: Shortcut): string {
  return [...shortcut.modifiers, shortcut.key].join('+')
}
