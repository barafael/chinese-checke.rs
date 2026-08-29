//! Tiny xorshift PRNG so the crate stays dependency-free. Prototype quality:
//! deterministic and good enough to shuffle test positions, nothing more.

pub struct Prng(u64);

impl Prng {
    pub const fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n`. Modulo bias is irrelevant here.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            (self.next_u64() >> 33) as u32 % n
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_and_in_range() {
        let a: Vec<u32> = (0..50)
            .map({
                let mut r = Prng::new(1);
                move |_| r.below(10)
            })
            .collect();
        let b: Vec<u32> = (0..50)
            .map({
                let mut r = Prng::new(1);
                move |_| r.below(10)
            })
            .collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|&v| v < 10));
        assert!(a.iter().any(|&v| v != a[0]), "should not be constant");
    }
}
