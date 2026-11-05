/**
 * `vocab_core::rng::Mulberry32`, the only source of randomness in exercise
 * generation.
 *
 * The generator is 32-bit so that both languages can run it bit for bit:
 * Rust's `wrapping_add`/`wrapping_mul` on `u32` are `+`/`Math.imul` followed
 * by `>>> 0` here. The same seed then gives the same shuffles, and the web
 * build shows the same exercise as the desktop build. Not cryptographic.
 */

/** `Mulberry32`: seeded with a `u32`. */
export class Mulberry32 {
  private state: number

  constructor(seed: number) {
    this.state = seed >>> 0
  }

  /** `next_u32`. */
  nextU32(): number {
    this.state = (this.state + 0x6d2b79f5) >>> 0
    let t = this.state
    t = Math.imul(t ^ (t >>> 15), t | 1) >>> 0
    t = (t ^ ((t + Math.imul(t ^ (t >>> 7), t | 61)) >>> 0)) >>> 0
    return (t ^ (t >>> 14)) >>> 0
  }

  /** `next_f64`: uniform in `[0, 1)`. */
  nextF64(): number {
    return this.nextU32() / 4_294_967_296
  }

  /** `below`: uniform integer in `[0, n)` for a `u32` `n`; 0 when `n` is 0. */
  below(n: number): number {
    if (n === 0) return 0
    return Math.floor(this.nextF64() * n)
  }

  /**
   * `shuffle`: Fisher–Yates from the end,
   * `for i in (1..len).rev() { j = below(i + 1); swap(i, j) }`.
   */
  shuffle<T>(items: T[]): void {
    for (let i = items.length - 1; i >= 1; i -= 1) {
      const j = this.below(i + 1)
      const item = items[i]
      items[i] = items[j]
      items[j] = item
    }
  }
}
