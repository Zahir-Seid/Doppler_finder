//! SRC-02 / SDD 6.2: Gaussian `(v, d)` event likelihood with
//! `Σ_vd + Σ_adsb + Σ_model` and `f0` marginalized through the fit
//! covariance sub-block.

#![allow(clippy::expect_used)]

use core_types::{AudioSeconds, EventId, Hertz, Metres, UtcMillis};
use events::EventFit;
use search::{ErrorBudget, Prediction, SearchError, event_log_likelihood};

fn fit(speed: f64, closest_m: f64, cov_vd: [[f64; 2]; 2]) -> EventFit {
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s: speed,
        closest_m: Metres(closest_m),
        heard_at: AudioSeconds(10.0),
        covariance: [
            [1e-4, 0.0, 0.0, 0.0],
            [0.0, cov_vd[0][0], cov_vd[0][1], 0.0],
            [0.0, cov_vd[1][0], cov_vd[1][1], 0.0],
            [0.0, 0.0, 0.0, 1e-2],
        ],
    }
}

fn prediction(speed: f64, slant_m: f64) -> Prediction {
    Prediction {
        emission_utc: UtcMillis(1_700_000_000_000),
        heard_utc: UtcMillis(1_700_000_010_000),
        slant_range_m: slant_m,
        speed_m_per_s: speed,
        observer_height_m: 100.0,
    }
}

#[test]
fn exact_match_scores_higher_than_mismatch() {
    let fit = fit(40.0, 2_000.0, [[1.0, 0.0], [0.0, 100.0]]);
    let budget = ErrorBudget::default();
    let good =
        event_log_likelihood(&fit, &prediction(40.0, 2_000.0), &budget).expect("finite inputs");
    let bad =
        event_log_likelihood(&fit, &prediction(55.0, 2_500.0), &budget).expect("finite inputs");
    assert!(good > bad, "exact match {good} should beat mismatch {bad}");
}

#[test]
fn likelihood_is_gaussian_in_the_residual() {
    // With zero fit covariance and a known budget, a residual r has
    // logL = -0.5 (rᵀΣ⁻¹r + ln det Σ + 2 ln 2π). Check the quadratic term by
    // comparing two residuals that differ only in the distance channel.
    let zero = [[0.0, 0.0], [0.0, 0.0]];
    let fit = fit(40.0, 2_000.0, zero);
    let budget = ErrorBudget {
        adsb: [[0.0, 0.0], [0.0, 0.0]],
        model: [[4.0, 0.0], [0.0, 400.0]],
    };
    // r = (0, 20 m): maha = 400/400 = 1.
    let near = event_log_likelihood(&fit, &prediction(40.0, 2_020.0), &budget).expect("ok");
    // r = (0, 40 m): maha = 1600/400 = 4.
    let far = event_log_likelihood(&fit, &prediction(40.0, 2_040.0), &budget).expect("ok");
    // Same normalization, so the difference is exactly -0.5*(4-1) = -1.5.
    assert!((near - far - 1.5).abs() < 1e-9, "near {near}, far {far}");
}

#[test]
fn correlated_fit_covariance_is_used() {
    // A strong positive v-d correlation in the fit covariance must change the
    // score versus the diagonal-only case — this catches a dropped sub-block
    // or transposition bug.
    let diagonal = fit(40.0, 2_000.0, [[4.0, 0.0], [0.0, 400.0]]);
    let correlated = fit(40.0, 2_000.0, [[4.0, 30.0], [30.0, 400.0]]);
    let budget = ErrorBudget::default();
    // Residual aligned with the correlation direction (v and d both high).
    let prediction = prediction(43.0, 2_030.0);
    let score_diagonal = event_log_likelihood(&diagonal, &prediction, &budget).expect("ok");
    let score_correlated = event_log_likelihood(&correlated, &prediction, &budget).expect("ok");
    assert!(
        (score_diagonal - score_correlated).abs() > 1e-6,
        "correlation must matter: {score_diagonal} vs {score_correlated}"
    );
}

#[test]
fn non_positive_definite_covariance_is_rejected() {
    // -100 variance in the v channel cannot be rescued by the provisional
    // budget, so the total covariance stays indefinite.
    let fit = fit(40.0, 2_000.0, [[-100.0, 0.0], [0.0, 100.0]]);
    assert!(matches!(
        event_log_likelihood(&fit, &prediction(40.0, 2_000.0), &ErrorBudget::default()),
        Err(SearchError::InvalidCovariance)
    ));
}

#[test]
fn nan_inputs_are_rejected() {
    let fit = fit(f64::NAN, 2_000.0, [[1.0, 0.0], [0.0, 100.0]]);
    assert!(matches!(
        event_log_likelihood(&fit, &prediction(40.0, 2_000.0), &ErrorBudget::default()),
        Err(SearchError::NotFinite)
    ));
}
