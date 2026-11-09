//! SDD 6.4 and FR-SL-04/NFR-01: the calibration factor `κ`.
//!
//! The Laplace covariance from [`crate::refine`] is only as honest as the
//! error budget behind it; `κ` rescales the reported position ellipse so
//! empirical coverage on simulation (and later held-out real clips, G3/T-6)
//! meets the nominal level. `κ` and the calibration version are stored
//! with every result via [`ModelVersions`].
//!
//! [`fit_kappa`] is the fitting procedure: given per-sample squared
//! Mahalanobis distances `d² = rᵀΣ⁻¹r` of the 2-D position ellipse,
//! `κ² = Q(d², p) / χ²₂(p)` where `Q` is the empirical `p`-quantile and
//! `χ²₂(p) = -2 ln(1 - p)` is the exact chi-square quantile for two degrees
//! of freedom (the ellipse lives in the `(east, north)` plane). Until the
//! simulator regression suite (FR-SIM-02/04) produces real `d²` samples,
//! [`PROVISIONAL_KAPPA`] stays at the uncalibrated value 1.0.

use core_types::manifest::ModelVersions;

use crate::error::SearchError;

/// Uncalibrated `κ`: the Laplace ellipse is reported as-is.
///
/// PROVISIONAL(SRC-09/T-6): replace with the value fitted on simulator
/// samples once FR-SIM-02/04 land; real-clip validation stays held out.
pub const PROVISIONAL_KAPPA: f64 = 1.0;

/// Calibration version tag stored beside `κ` in every result manifest.
pub const PROVISIONAL_CALIBRATION_VERSION: &str = "src09-provisional-v1";

/// The calibration state attached to a result: the ellipse scale `κ` and
/// the version of the calibration procedure/model that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    /// Ellipse semi-axis scale; `κ = 1` reports the raw Laplace ellipse.
    /// Must be finite and positive.
    pub kappa: f64,
    /// Calibration procedure version, e.g. [`PROVISIONAL_CALIBRATION_VERSION`].
    pub version: String,
}

impl Calibration {
    /// The provisional uncalibrated state.
    pub fn provisional() -> Self {
        Self {
            kappa: PROVISIONAL_KAPPA,
            version: PROVISIONAL_CALIBRATION_VERSION.to_owned(),
        }
    }

    /// A calibrated state; `kappa` must be finite and positive.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::NotFinite`] for a non-finite or non-positive
    /// `kappa`.
    pub fn new(kappa: f64, version: impl Into<String>) -> Result<Self, SearchError> {
        if !kappa.is_finite() || kappa <= 0.0 {
            return Err(SearchError::NotFinite);
        }
        Ok(Self {
            kappa,
            version: version.into(),
        })
    }

    /// The `(east, north)` position block of a 3×3 `(e, n, s)` covariance,
    /// scaled by `κ²` — the calibrated confidence ellipse (FR-SL-04).
    pub fn ellipse_covariance(&self, covariance: &[[f64; 3]; 3]) -> [[f64; 2]; 2] {
        let k2 = self.kappa * self.kappa;
        [
            [covariance[0][0] * k2, covariance[0][1] * k2],
            [covariance[1][0] * k2, covariance[1][1] * k2],
        ]
    }

    /// The manifest record carrying `κ` and this calibration version
    /// (SDD 9.4 / NFR-04).
    pub fn model_versions(&self, model: &str) -> ModelVersions {
        ModelVersions {
            model: model.to_owned(),
            calibration: self.version.clone(),
            kappa: self.kappa,
        }
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::provisional()
    }
}

/// Fit `κ` so the empirical coverage of `d2` samples at `coverage` matches
/// the nominal level.
///
/// `d2` holds per-sample squared Mahalanobis distances of position errors
/// under the *uncalibrated* ellipse; the returned `κ` scales the ellipse so
/// that a fraction `coverage` of samples satisfies `d2 / κ² ≤ χ²₂(coverage)`.
/// Equivalently `κ² = Q_p(d2) / (-2 ln(1 - p))` with `Q_p` the empirical
/// `p`-quantile (nearest-rank).
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for an empty slice, non-finite or
/// negative samples, or `coverage` outside `(0, 1)`.
pub fn fit_kappa(d2: &[f64], coverage: f64) -> Result<f64, SearchError> {
    if d2.is_empty()
        || d2.iter().any(|v| !v.is_finite() || *v < 0.0)
        || !coverage.is_finite()
        || coverage <= 0.0
        || coverage >= 1.0
    {
        return Err(SearchError::NotFinite);
    }
    let mut sorted = d2.to_vec();
    sorted.sort_by(f64::total_cmp);
    // Nearest-rank quantile: index ceil(p·n) − 1.
    let rank = (coverage * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    let q = sorted.get(index).copied().unwrap_or(0.0);
    // χ²₂(p) = -2 ln(1 - p), exact for the 2-D ellipse.
    let chi2 = -2.0 * (1.0 - coverage).ln();
    let kappa2 = q / chi2;
    if !kappa2.is_finite() || kappa2 <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    Ok(kappa2.sqrt())
}
