//! Analysis windows for the STFT (SDD section 5).

/// Window shape used for spectral analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowKind {
    /// All ones. Used only for closed-form tests.
    Rectangular,
    /// Symmetric Hann window.
    Hann,
}

/// Build a window of `len` samples.
///
/// Returns an empty vector for `len == 0`; a single-sample window is `[1.0]`.
/// The Hann window is periodic-symmetric, matching the usual STFT definition.
pub fn window(kind: WindowKind, len: usize) -> Vec<f64> {
    match kind {
        WindowKind::Rectangular => vec![1.0; len],
        WindowKind::Hann => {
            if len <= 1 {
                return vec![1.0; len];
            }
            let denominator = (len - 1) as f64;
            (0..len)
                .map(|index| {
                    let phase = std::f64::consts::TAU * index as f64 / denominator;
                    0.5 - 0.5 * phase.cos()
                })
                .collect()
        }
    }
}
