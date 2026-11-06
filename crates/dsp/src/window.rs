//! Analysis windows for the STFT (SDD section 5).

use crate::error::DspError;

/// Kaiser shape parameter for roughly 80 dB sidelobe attenuation.
///
/// Kaiser's design formula for a sidelobe level `A` above 50 dB is
/// `beta = 0.1102 (A - 8.7)` (Harris 1978), so `A = 80 dB` gives `7.86`. The
/// analysis window wants the same sidelobe level as the anti-alias filter, so
/// leakage from a strong propeller harmonic cannot mask a weak one nearby.
pub const DEFAULT_KAISER_BETA: f64 = 7.86;

/// Largest number of terms used in the modified Bessel series.
const BESSEL_MAX_TERMS: usize = 200;

/// Relative convergence threshold for the modified Bessel series.
const BESSEL_EPSILON: f64 = 1.0e-16;

/// Window shape used for spectral analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowKind {
    /// All ones. Used only for closed-form tests.
    Rectangular,
    /// Symmetric Hann window.
    Hann,
    /// Kaiser window with the given shape parameter.
    ///
    /// `beta` trades main-lobe width against sidelobe level; see
    /// [`DEFAULT_KAISER_BETA`] for the analysis default.
    Kaiser {
        /// Kaiser shape parameter, finite and non-negative.
        beta: f64,
    },
}

/// Build a window of `len` samples.
///
/// Returns an empty vector for `len == 0`; a single-sample window is `[1.0]`.
/// The Kaiser window is `I0(beta * sqrt(1 - (2n/(N-1) - 1)^2)) / I0(beta)`
/// (SDD section 5), which is symmetric and peaks at one in the centre.
///
/// # Errors
///
/// Returns [`DspError::InvalidWindow`] when a Kaiser shape parameter is NaN,
/// infinite or negative.
pub fn window(kind: WindowKind, len: usize) -> Result<Vec<f64>, DspError> {
    match kind {
        WindowKind::Rectangular => Ok(vec![1.0; len]),
        WindowKind::Hann => {
            if len <= 1 {
                return Ok(vec![1.0; len]);
            }
            let denominator = (len - 1) as f64;
            Ok((0..len)
                .map(|index| {
                    let phase = std::f64::consts::TAU * index as f64 / denominator;
                    0.5 - 0.5 * phase.cos()
                })
                .collect())
        }
        WindowKind::Kaiser { beta } => kaiser_window(beta, len),
    }
}

/// Build a symmetric Kaiser window.
fn kaiser_window(beta: f64, len: usize) -> Result<Vec<f64>, DspError> {
    if !beta.is_finite() || beta < 0.0 {
        return Err(DspError::InvalidWindow { beta });
    }
    if len <= 1 {
        return Ok(vec![1.0; len]);
    }
    let denominator = bessel_i0(beta);
    if !denominator.is_finite() || denominator <= 0.0 {
        return Err(DspError::InvalidWindow { beta });
    }
    let centre = (len - 1) as f64 / 2.0;
    Ok((0..len)
        .map(|index| {
            let ratio = (index as f64 - centre) / centre;
            let taper = (1.0 - ratio * ratio).max(0.0).sqrt();
            bessel_i0(beta * taper) / denominator
        })
        .collect())
}

/// Modified Bessel function of the first kind, order zero.
///
/// Evaluated as the series `sum_k (x/2)^(2k) / (k!)^2`, which converges in a
/// few dozen terms for the shape parameters used here. Shared with the Kaiser
/// FIR design in [`crate::frontend`].
pub(crate) fn bessel_i0(value: f64) -> f64 {
    let half = value / 2.0;
    let half_squared = half * half;
    let mut term = 1.0;
    let mut sum = 1.0;
    for order in 1..=BESSEL_MAX_TERMS {
        let order_f = order as f64;
        term *= half_squared / (order_f * order_f);
        sum += term;
        if term.abs() <= sum.abs() * BESSEL_EPSILON {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_KAISER_BETA, WindowKind, bessel_i0, window};
    use crate::error::DspError;

    #[test]
    fn bessel_i0_at_zero_is_one() {
        assert!((bessel_i0(0.0) - 1.0).abs() < 1.0e-15);
    }

    #[test]
    fn kaiser_window_is_symmetric_and_peaks_at_one() {
        let taps = window(
            WindowKind::Kaiser {
                beta: DEFAULT_KAISER_BETA,
            },
            101,
        )
        .expect("kaiser window");
        for index in 0..taps.len() {
            let mirrored = taps.len() - 1 - index;
            let (left, right) = (taps[index], taps[mirrored]);
            assert!((left - right).abs() < 1.0e-12, "index {index}");
        }
        let centre = taps[50];
        assert!((centre - 1.0).abs() < 1.0e-12, "centre {centre}");
        // The endpoint is 1/I0(beta), about 2.7e-3 for the 80 dB default, so
        // the 80 dB sidelobe claim is relative to the unit peak.
        let edge = taps[0];
        let expected = 1.0 / bessel_i0(DEFAULT_KAISER_BETA);
        assert!((edge - expected).abs() < 1.0e-12, "edge {edge}");
        assert!(edge < 0.01, "edge {edge}");
    }

    #[test]
    fn kaiser_window_rejects_a_negative_beta() {
        let result = window(WindowKind::Kaiser { beta: -1.0 }, 16);
        assert!(matches!(result, Err(DspError::InvalidWindow { .. })));
    }

    #[test]
    fn kaiser_window_rejects_a_non_finite_beta() {
        let result = window(
            WindowKind::Kaiser {
                beta: f64::INFINITY,
            },
            16,
        );
        assert!(matches!(result, Err(DspError::InvalidWindow { .. })));
    }

    #[test]
    fn hann_window_has_zero_endpoints() {
        let taps = window(WindowKind::Hann, 65).expect("hann window");
        assert!(taps[0].abs() < 1.0e-15, "start {}", taps[0]);
        assert!(taps[64].abs() < 1.0e-15, "end {}", taps[64]);
    }

    #[test]
    fn zero_length_window_is_empty() {
        let taps = window(WindowKind::Rectangular, 0).expect("empty window");
        assert_eq!(taps, Vec::<f64>::new());
    }
}
