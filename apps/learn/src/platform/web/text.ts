/**
 * Rust's string semantics, where they differ from JavaScript's.
 *
 * The core compares, trims and splits text with Rust's `str` methods, and
 * the port must draw the same lines, or an exercise built in the browser
 * shows a different option than the desktop one for the same seed:
 *
 * - **Whitespace** is Unicode `White_Space` (`char::is_whitespace`). It
 *   includes U+0085 (NEL), which `String.prototype.trim` keeps, and excludes
 *   U+FEFF (the byte-order mark), which `trim` removes.
 * - **Characters** are code points, not UTF-16 code units: a supplementary
 *   ideograph is one `char`.
 * - **Letters and digits** are `char::is_alphanumeric`: the `Alphabetic`
 *   property or a general category of `N*`.
 * - **Ordering** of `String`s is byte order of UTF-8, which is code point
 *   order, not the UTF-16 order of `<`.
 *
 * Lowercasing needs nothing here: `toLowerCase` and `str::to_lowercase` both
 * apply Unicode's full default mapping, final sigma included.
 */

/** The body of a regular-expression class matching `White_Space`. */
export const WHITE_SPACE =
  '\\t\\n\\v\\f\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000'
const IS_WHITE_SPACE = new RegExp(`^[${WHITE_SPACE}]$`)
const LEADING = new RegExp(`^[${WHITE_SPACE}]+`)
const TRAILING = new RegExp(`[${WHITE_SPACE}]+$`)
const RUNS = new RegExp(`[${WHITE_SPACE}]+`)
const ALPHANUMERIC = /^[\p{Alphabetic}\p{N}]$/u
const GRAPHEME_EXTEND = /^\p{Grapheme_Extend}$/u
const NOT_PRINTABLE = /^[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Cn}\p{Zl}\p{Zp}\p{Zs}]$/u

/** `char::is_whitespace` of one code point. */
export function isWhitespace(c: string): boolean {
  return IS_WHITE_SPACE.test(c)
}

/** `str::trim`. */
export function trim(text: string): string {
  return trimEnd(trimStart(text))
}

/** `str::trim_start`. */
export function trimStart(text: string): string {
  return text.replace(LEADING, '')
}

/** `str::trim_end`. */
export function trimEnd(text: string): string {
  return text.replace(TRAILING, '')
}

/** `str::split_whitespace`: the non-empty runs between whitespace. */
export function splitWhitespace(text: string): string[] {
  return text.split(RUNS).filter((part) => part !== '')
}

/** Whether `text` contains any whitespace. */
export function hasWhitespace(text: string): boolean {
  return chars(text).some(isWhitespace)
}

/** `str::chars`: the code points of `text`. */
export function chars(text: string): string[] {
  return Array.from(text)
}

/** `char::is_alphanumeric` of one code point. */
export function isAlphanumeric(c: string): boolean {
  return ALPHANUMERIC.test(c)
}

/** `Ord for str`: code point order. */
export function compareStrings(a: string, b: string): number {
  const left = chars(a)
  const right = chars(b)
  const shared = Math.min(left.length, right.length)
  for (let index = 0; index < shared; index += 1) {
    const difference = codePoint(left[index]) - codePoint(right[index])
    if (difference !== 0) return difference
  }
  return left.length - right.length
}

/** The code point of a one-character string. */
export function codePoint(c: string): number {
  return c.codePointAt(0) ?? 0
}

/**
 * `{:?}` of a `str`: in double quotes, with `\0 \t \r \n \\ \"` escaped and
 * every grapheme extender or character that is not printable (controls,
 * format characters, unassigned code points, separators other than the
 * space) written as `\u{hex}`.
 */
export function debugString(text: string): string {
  let out = '"'
  for (const c of text) {
    const escape = ESCAPES[c]
    if (escape !== undefined) out += escape
    else if (c !== ' ' && (GRAPHEME_EXTEND.test(c) || NOT_PRINTABLE.test(c))) {
      out += `\\u{${codePoint(c).toString(16)}}`
    } else out += c
  }
  return `${out}"`
}

const ESCAPES: Record<string, string> = {
  '\0': '\\0',
  '\t': '\\t',
  '\r': '\\r',
  '\n': '\\n',
  '\\': '\\\\',
  '"': '\\"',
}
