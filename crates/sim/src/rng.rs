//! Seeded random numbers for the simulator.
//!
//! Norms section 5 requires `rand_chacha` generators seeded from the scenario,
//! so a fixed scenario gives byte-identical audio and feeds. Nothing here is
//! thread-dependent: one generator per stream, consumed in a fixed order.

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;

/// Two raised to the fifty-third power, the scale of a unit-interval draw.
const TWO_POW_53: f64 = 9_007_199_254_740_992.0;

/// A seeded source of uniform and normal deviates.
pub(crate) struct NormalSource {
    rng: ChaCha20Rng,
}

impl NormalSource {
    /// Open the stream for a scenario seed.
    pub(crate) fn seeded(seed: u64) -> Self {
        Self {
            rng: ChaCha20Rng::seed_from_u64(seed),
        }
    }

    /// A draw from `[0, 1)` with 53 bits of resolution.
    pub(crate) fn uniform(&mut self) -> f64 {
        let bits = self.rng.next_u64() >> 11;
        bits as f64 / TWO_POW_53
    }

    /// A draw from `(0, 1]`, so its logarithm is finite.
    fn uniform_open(&mut self) -> f64 {
        1.0 - self.uniform()
    }

    /// A standard normal deviate by the Box–Muller transform.
    ///
    /// Box–Muller rather than a distribution crate keeps one dependency out of
    /// the build; the transform is exact in exact arithmetic, and only its two
    /// transcendental calls differ between platforms in the last bits.
    pub(crate) fn standard_normal(&mut self) -> f64 {
        let first = self.uniform_open();
        let second = self.uniform();
        (-2.0 * first.ln()).sqrt() * (core::f64::consts::TAU * second).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_stream_is_reproducible() {
        let mut left = NormalSource::seeded(3);
        let mut right = NormalSource::seeded(3);
        for _ in 0..8 {
            assert_eq!(left.uniform(), right.uniform());
            assert_eq!(left.standard_normal(), right.standard_normal());
        }
    }

    #[test]
    fn different_seeds_diverge_immediately() {
        let mut third = NormalSource::seeded(3);
        let mut fourth = NormalSource::seeded(4);
        assert!((third.uniform() - fourth.uniform()).abs() > 1e-9);
    }

    #[test]
    fn normal_deviates_have_unit_scale_and_no_drift() {
        let mut source = NormalSource::seeded(11);
        let count = 20_000;
        let mut sum = 0.0;
        let mut squares = 0.0;
        for _ in 0..count {
            let value = source.standard_normal();
            sum += value;
            squares += value * value;
        }
        let mean = sum / f64::from(count);
        let variance = squares / f64::from(count) - mean * mean;
        // 20 000 draws put the standard error of the mean at 0.007 and of the
        // variance at 0.01, so these bounds are about seven sigma.
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.07, "variance {variance}");
    }
}
