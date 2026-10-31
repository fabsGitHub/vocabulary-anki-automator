//! Deterministic, dependency-free PRNG for exercise generation.
//!
//! Mulberry32 was chosen because it is 32-bit and therefore trivially
//! identical in Rust and TypeScript (`Math.imul`), which lets the Learn web
//! build reproduce exactly the same shuffles as the Rust core for a given
//! seed. Not cryptographic; never use it for anything security-related.

#[derive(Debug, Clone)]
pub struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_add(0x6D2B_79F5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }

    /// Uniform float in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }

    /// Uniform integer in `[0, n)`. Returns 0 when `n == 0`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (self.next_f64() * f64::from(n)) as u32
    }

    /// In-place Fisher–Yates shuffle, walking from the end:
    /// `for i in (1..len).rev() { j = below(i + 1); swap(i, j) }`.
    /// The TypeScript port must use exactly this order.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden vectors produced by the canonical JavaScript implementation.
    /// The Learn web port asserts the same numbers.
    #[test]
    fn matches_reference_javascript_outputs() {
        let cases: [(u32, [u32; 5]); 4] = [
            (0, [1144304738, 1416247, 958946056, 627933444, 2007157716]),
            (
                1,
                [2693262067, 11749833, 2265367787, 4213581821, 4159151403],
            ),
            (
                42,
                [2581720956, 1925393290, 3661312704, 2876485805, 750819978],
            ),
            (
                4294967295,
                [3850105811, 813802916, 3073704848, 4054706436, 3630262831],
            ),
        ];
        for (seed, expected) in cases {
            let mut rng = Mulberry32::new(seed);
            let got: Vec<u32> = (0..5).map(|_| rng.next_u32()).collect();
            assert_eq!(got, expected, "seed {seed}");
        }
    }

    #[test]
    fn below_stays_in_range_and_shuffle_is_a_permutation() {
        let mut rng = Mulberry32::new(7);
        for _ in 0..1000 {
            assert!(rng.below(5) < 5);
        }
        assert_eq!(rng.below(0), 0);
        let mut items: Vec<u32> = (0..20).collect();
        rng.shuffle(&mut items);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
    }
}
