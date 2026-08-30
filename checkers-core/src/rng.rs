//! A tiny xorshift PRNG for deterministic test inputs.
//!
//! `checkers-core` deliberately depends only on `linkme`, so the law sample
//! generators and downstream tests share this local helper rather than pulling
//! a crate. Prototype quality: reproducible across runs, uniform enough for
//! sampling test positions, nothing more.

/// xorshift64*. Fixed seed rather than a random one, so a failure is
/// reproducible.
pub struct Xorshift(u64);

impl Xorshift {
    pub const fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n`. Modulo bias is irrelevant for sampling.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() >> 33) as usize % n
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_and_in_range() {
        let mut a = Xorshift::new(1);
        let mut b = Xorshift::new(1);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Xorshift::new(7);
        assert!((0..200).all(|_| r.below(10) < 10));
    }

    #[test]
    fn a_zero_seed_does_not_stick_at_zero() {
        let mut r = Xorshift::new(0);
        assert_ne!(r.next_u64(), 0);
    }
}
