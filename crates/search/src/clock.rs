//! SDD 6.2 clock term: events must agree on the recording start `s`
//! (FR-SL-01).
//!
//! Each event implies a recording start
//! `s_i = t_u(heard closest approach) − t_a,i`, where `t_u` is the predicted
//! heard time as UTC and `t_a,i` is the fitted heard time on the audio
//! clock. Under a hypothesis `(x, s)` every event contributes one Gaussian
//! `log N(s; s_i, σ²_s,i)`; `σ_s,i` combines the fit's timing variance
//! (`covariance[3][3]`, the `heard_time_s` parameter) with the ADS-B timing
//! error. A known recording time adds one more Gaussian term with its own
//! sigma. This replaces the hard offset filter of earlier designs.
//!
//! All times here are seconds: `s` and `s_i` are Unix seconds, `t_a` is
//! audio seconds since the recording start.

use events::EventFit;

use crate::error::SearchError;
use crate::predict::Prediction;

/// ADS-B-side timing standard deviation in seconds.
///
/// PROVISIONAL(T-4/SRC-03): receiver timestamp jitter and report latency
/// are sub-second to a few seconds depending on source; refine from the
/// measured latency statistics recorded per ingest (FR-DA-01) once real
/// runs exist.
pub const PROVISIONAL_ADSB_TIMING_SIGMA_S: f64 = 1.0;

/// The recording start implied by one `(fit, prediction)` pair, in Unix
/// seconds: `s_i = heard_utc − heard_at`.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] when either time is non-finite or the
/// prediction's heard UTC is missing.
pub fn implied_recording_start(
    fit: &EventFit,
    prediction: &Prediction,
) -> Result<f64, SearchError> {
    let heard_unix_s = prediction.heard_utc.0 as f64 / 1_000.0;
    if !heard_unix_s.is_finite() || !fit.heard_at.0.is_finite() {
        return Err(SearchError::NotFinite);
    }
    Ok(heard_unix_s - fit.heard_at.0)
}

/// Timing standard deviation `σ_s,i` for one event, in seconds.
///
/// Combines the fit's `heard_time_s` variance (`covariance[3][3]`) with the
/// ADS-B timing sigma in quadrature. The fit variance is in audio-second
/// units squared; both terms are seconds so they add directly.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] when the fit variance is negative or
/// non-finite, or `adsb_timing_sigma_s` is not finite and non-negative.
pub fn recording_start_sigma(fit: &EventFit, adsb_timing_sigma_s: f64) -> Result<f64, SearchError> {
    let fit_variance = fit.covariance[3][3];
    if !fit_variance.is_finite() || fit_variance < 0.0 {
        return Err(SearchError::NotFinite);
    }
    if !adsb_timing_sigma_s.is_finite() || adsb_timing_sigma_s < 0.0 {
        return Err(SearchError::NotFinite);
    }
    Ok((fit_variance + adsb_timing_sigma_s * adsb_timing_sigma_s).sqrt())
}

/// Clock log-likelihood of one event under recording start `s_unix_s`:
/// `log N(s; s_i, σ²_s,i)`, including the normalization constant.
///
/// # Errors
///
/// Propagates the errors of [`implied_recording_start`] and
/// [`recording_start_sigma`].
pub fn clock_log_likelihood(
    fit: &EventFit,
    prediction: &Prediction,
    s_unix_s: f64,
    adsb_timing_sigma_s: f64,
) -> Result<f64, SearchError> {
    if !s_unix_s.is_finite() {
        return Err(SearchError::NotFinite);
    }
    let s_i = implied_recording_start(fit, prediction)?;
    let sigma = recording_start_sigma(fit, adsb_timing_sigma_s)?;
    if sigma <= 0.0 {
        return Err(SearchError::InvalidCovariance);
    }
    let z = (s_unix_s - s_i) / sigma;
    Ok(-0.5 * (z * z + std::f64::consts::TAU.ln()))
}

/// The extra Gaussian term a known recording time contributes:
/// `log N(s; s_known_unix_s, σ²_known)`.
///
/// Pass `sigma_known_s = 0.0` only when the time is exact; in practice the
/// known time still carries clock and rounding uncertainty, so callers
/// should pass a small positive sigma.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for non-finite inputs or a negative
/// sigma.
pub fn known_time_log_likelihood(
    s_unix_s: f64,
    s_known_unix_s: f64,
    sigma_known_s: f64,
) -> Result<f64, SearchError> {
    if !s_unix_s.is_finite() || !s_known_unix_s.is_finite() {
        return Err(SearchError::NotFinite);
    }
    if !sigma_known_s.is_finite() || sigma_known_s <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    let z = (s_unix_s - s_known_unix_s) / sigma_known_s;
    Ok(-0.5 * (z * z + std::f64::consts::TAU.ln()))
}

/// Sum the clock terms of every event under `s_unix_s` — the `Σ_i` part of
/// the SDD 6.2 clock contribution for one assignment.
///
/// # Errors
///
/// Propagates the first error from any event's term.
pub fn clock_terms_log_likelihood(
    fits_and_predictions: &[(EventFit, Prediction)],
    s_unix_s: f64,
    adsb_timing_sigma_s: f64,
) -> Result<f64, SearchError> {
    let mut total = 0.0;
    for (fit, prediction) in fits_and_predictions {
        total += clock_log_likelihood(fit, prediction, s_unix_s, adsb_timing_sigma_s)?;
    }
    Ok(total)
}
