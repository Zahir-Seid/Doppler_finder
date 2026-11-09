//! SDD 6.2 event likelihood: Gaussian residual on `(v, d)` (FR-SL-01).
//!
//! Each event contributes `L_i`, a Gaussian on the residual between the fitted
//! `(speed, closest distance)` and the prediction for a candidate
//! `(observer, flight)` pair. The covariance is
//! `Σ_vd + Σ_adsb + Σ_model`:
//!
//! - `Σ_vd` is the `(v, d)` marginal of the event fit's 4×4 covariance —
//!   taking the sub-block marginalizes the nuisance `f0` exactly, because the
//!   fit was joint in `(f0, v, d, t)`;
//! - `Σ_adsb` covers track-side errors (sampling, quantization, altitude
//!   datum, latency) propagated into `(v, d)`;
//! - `Σ_model` covers model errors (manoeuvres, wind, sound speed) that make
//!   even a perfect track predict the wrong `(v, d)`.
//!
//! `Σ_adsb` and `Σ_model` have no measured values until the FR-SIM-02 error
//! budget lands; [`ErrorBudget::default`] carries `PROVISIONAL` diagonal
//! values chosen to be conservative for propeller legs and must be replaced
//! by measured statistics before G1/G2 sign-off.

use events::EventFit;

use crate::error::SearchError;
use crate::predict::Prediction;

/// `(v, d)` standard deviations from the ADS-B side, in `(m/s, m)`.
///
/// PROVISIONAL(T-4/SRC-02): sampling jitter and position quantization
/// dominate; refine from measured archive statistics (FR-SIM-02).
const PROVISIONAL_ADSB_SIGMA: [f64; 2] = [3.0, 60.0];

/// `(v, d)` standard deviations from the model side, in `(m/s, m)`.
///
/// PROVISIONAL(T-4/SRC-02): manoeuvres inside a pass and wind error are the
/// leading terms; refine from the simulator error budget (FR-SIM-02).
const PROVISIONAL_MODEL_SIGMA: [f64; 2] = [2.0, 40.0];

/// Track-side and model-side `(v, d)` error covariance, in `(m/s, m)` units.
///
/// Both matrices must be symmetric positive semi-definite; the sum with
/// `Σ_vd` must be positive definite for the Gaussian to be normalizable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErrorBudget {
    /// Covariance of track-side prediction error on `(v, d)`.
    pub adsb: [[f64; 2]; 2],
    /// Covariance of model error on `(v, d)`.
    pub model: [[f64; 2]; 2],
}

impl Default for ErrorBudget {
    /// Provisional diagonal budget pending the FR-SIM-02 measurements.
    fn default() -> Self {
        Self {
            adsb: diagonal(PROVISIONAL_ADSB_SIGMA),
            model: diagonal(PROVISIONAL_MODEL_SIGMA),
        }
    }
}

fn diagonal(sigma: [f64; 2]) -> [[f64; 2]; 2] {
    [[sigma[0] * sigma[0], 0.0], [0.0, sigma[1] * sigma[1]]]
}

/// Log-likelihood of one event under one `(observer, flight)` prediction.
///
/// The residual is `(v_fit − v_pred, d_fit − d_pred)` in `(m/s, m)`. `f0` is
/// a nuisance parameter: it is marginalized by taking the `(v, d)` sub-block
/// of the joint fit covariance, which already carries its uncertainty.
///
/// Returns the full normalized log Gaussian, including the `log det Σ` and
/// `2 log 2π` terms, so events with different covariances compare correctly
/// in the fused posterior.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] when any input is non-finite,
/// [`SearchError::InvalidCovariance`] when `Σ_vd + Σ_adsb + Σ_model` is not
/// positive definite, or [`SearchError::Adsb`] when the fit's covariance is
/// unusable.
pub fn event_log_likelihood(
    fit: &EventFit,
    prediction: &Prediction,
    budget: &ErrorBudget,
) -> Result<f64, SearchError> {
    let gaussian = vd_gaussian(fit, prediction, budget)?;
    let maha = gaussian.mahalanobis_squared();
    // Full bivariate Gaussian: -0.5 (rᵀΣ⁻¹r + ln det Σ + 2 ln 2π).
    Ok(-0.5 * (maha + gaussian.log_determinant + 2.0 * std::f64::consts::TAU.ln()))
}

/// The `(v, d)` Gaussian shared by the event likelihood and the SRC-05 gate.
pub(crate) struct VdGaussian {
    inverse: [[f64; 2]; 2],
    residual: [f64; 2],
    log_determinant: f64,
}

impl VdGaussian {
    /// Squared Mahalanobis distance `rᵀΣ⁻¹r` of the `(v, d)` residual.
    pub(crate) fn mahalanobis_squared(&self) -> f64 {
        quadratic_form(self.inverse, self.residual)
    }
}

/// Build the `(v, d)` Gaussian for one `(fit, prediction)` pair.
///
/// # Errors
///
/// Same as [`event_log_likelihood`].
pub(crate) fn vd_gaussian(
    fit: &EventFit,
    prediction: &Prediction,
    budget: &ErrorBudget,
) -> Result<VdGaussian, SearchError> {
    if !fit.speed_m_per_s.is_finite()
        || !fit.closest_m.0.is_finite()
        || !prediction.speed_m_per_s.is_finite()
        || !prediction.slant_range_m.is_finite()
    {
        return Err(SearchError::NotFinite);
    }
    let sigma_vd = fit_covariance_subblock(fit)?;
    let total = add(add(sigma_vd, budget.adsb)?, budget.model)?;
    let inverse = invert_2x2(total).ok_or(SearchError::InvalidCovariance)?;
    let determinant = determinant_2x2(total);
    if determinant <= 0.0 {
        return Err(SearchError::InvalidCovariance);
    }
    let residual = [
        fit.speed_m_per_s - prediction.speed_m_per_s,
        fit.closest_m.0 - prediction.slant_range_m,
    ];
    Ok(VdGaussian {
        inverse,
        residual,
        log_determinant: determinant.ln(),
    })
}

/// Validate that `Σ_vd + Σ_adsb + Σ_model` is positive definite for `fit`,
/// without needing a prediction. The determinant depends only on the
/// covariance, so a failure here means every cell would fail identically —
/// callers should check once per event rather than skip silently.
///
/// # Errors
///
/// Same covariance errors as [`event_log_likelihood`].
pub(crate) fn check_fit_covariance(
    fit: &EventFit,
    budget: &ErrorBudget,
) -> Result<(), SearchError> {
    let sigma_vd = fit_covariance_subblock(fit)?;
    let total = add(add(sigma_vd, budget.adsb)?, budget.model)?;
    if determinant_2x2(total) <= 0.0 {
        return Err(SearchError::InvalidCovariance);
    }
    Ok(())
}

/// `(v, d)` marginal of the fit covariance: parameter indices 1 and 2 of
/// `(f0, v, d, t)`, in `(m/s, m)` units.
fn fit_covariance_subblock(fit: &EventFit) -> Result<[[f64; 2]; 2], SearchError> {
    let covariance = fit.covariance;
    let block = [
        [covariance[1][1], covariance[1][2]],
        [covariance[2][1], covariance[2][2]],
    ];
    if block.iter().flatten().all(|value| value.is_finite()) {
        Ok(block)
    } else {
        Err(SearchError::Adsb(
            "event fit covariance is not finite".to_owned(),
        ))
    }
}

fn add(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> Result<[[f64; 2]; 2], SearchError> {
    let sum = [
        [a[0][0] + b[0][0], a[0][1] + b[0][1]],
        [a[1][0] + b[1][0], a[1][1] + b[1][1]],
    ];
    if sum.iter().flatten().all(|value| value.is_finite()) {
        Ok(sum)
    } else {
        Err(SearchError::NotFinite)
    }
}

fn determinant_2x2(m: [[f64; 2]; 2]) -> f64 {
    m[0][0] * m[1][1] - m[0][1] * m[1][0]
}

fn invert_2x2(m: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let determinant = determinant_2x2(m);
    if determinant <= 0.0 || !determinant.is_finite() {
        return None;
    }
    Some([
        [m[1][1] / determinant, -m[0][1] / determinant],
        [-m[1][0] / determinant, m[0][0] / determinant],
    ])
}

fn quadratic_form(inverse: [[f64; 2]; 2], residual: [f64; 2]) -> f64 {
    let [r0, r1] = residual;
    let [[i00, i01], [i10, i11]] = inverse;
    r0 * (i00 * r0 + i01 * r1) + r1 * (i10 * r0 + i11 * r1)
}
