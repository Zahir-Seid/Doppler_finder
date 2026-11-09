//! SRC-03 / SDD 6.2: clock term — each event implies a recording start and
//! the terms must agree within `σ_s,i`; a known time adds one Gaussian.

#![allow(clippy::expect_used)]

use core_types::{AudioSeconds, EventId, Hertz, Metres, UtcMillis};
use events::EventFit;
use search::{
    PROVISIONAL_ADSB_TIMING_SIGMA_S, Prediction, SearchError, clock_log_likelihood,
    clock_terms_log_likelihood, implied_recording_start, known_time_log_likelihood,
    recording_start_sigma,
};

fn fit(heard_at_s: f64, heard_var_s2: f64) -> EventFit {
    let mut covariance = [[0.0; 4]; 4];
    covariance[3][3] = heard_var_s2;
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s: 40.0,
        closest_m: Metres(2_000.0),
        heard_at: AudioSeconds(heard_at_s),
        covariance,
    }
}

fn prediction(heard_utc_ms: i64) -> Prediction {
    Prediction {
        emission_utc: UtcMillis(heard_utc_ms - 9_000),
        heard_utc: UtcMillis(heard_utc_ms),
        slant_range_m: 2_000.0,
        speed_m_per_s: 40.0,
        observer_height_m: 100.0,
    }
}

#[test]
fn implied_start_is_heard_utc_minus_audio_time() {
    // heard at audio t=12.5 s, UTC 1_700_000_012_500 ms -> s_i = 1_700_000_000.
    let fit = fit(12.5, 0.25);
    let pred = prediction(1_700_000_012_500);
    let s_i = implied_recording_start(&fit, &pred).expect("finite");
    assert!((s_i - 1_700_000_000.0).abs() < 1e-9);
}

#[test]
fn sigma_combines_fit_and_adsb_timing() {
    // fit variance 0.25 s² + adsb sigma 1.0 s -> sqrt(1.25) ≈ 1.118 s.
    let fit = fit(12.5, 0.25);
    let sigma = recording_start_sigma(&fit, PROVISIONAL_ADSB_TIMING_SIGMA_S).expect("ok");
    assert!((sigma - 1.118_034).abs() < 1e-5);
}

#[test]
fn clock_term_peaks_at_implied_start() {
    let fit = fit(12.5, 0.25);
    let pred = prediction(1_700_000_012_500);
    let sigma = recording_start_sigma(&fit, PROVISIONAL_ADSB_TIMING_SIGMA_S).expect("ok");
    let at = clock_log_likelihood(&fit, &pred, 1_700_000_000.0, 1.0).expect("ok");
    let off = clock_log_likelihood(&fit, &pred, 1_700_000_000.0 + 5.0 * sigma, 1.0).expect("ok");
    // 5σ away: logL drops by 12.5 exactly (z²/2), up to fp rounding through
    // the irrational sigma.
    assert!((at - off - 12.5).abs() < 1e-6, "at {at}, off {off}");
    assert!(at > off);
}

#[test]
fn two_events_must_agree_on_s() {
    // Both events imply the same start when their heard times line up; a
    // candidate s far from it is penalised by both terms.
    let fit_a = fit(12.5, 0.25);
    let fit_b = fit(40.0, 0.25);
    let pred_a = prediction(1_700_000_012_500);
    let pred_b = prediction(1_700_000_040_000);
    let pairs = [(fit_a, pred_a), (fit_b, pred_b)];
    let on = clock_terms_log_likelihood(&pairs, 1_700_000_000.0, 1.0).expect("ok");
    let off = clock_terms_log_likelihood(&pairs, 1_700_000_100.0, 1.0).expect("ok");
    assert!(on > off);
}

#[test]
fn known_time_adds_one_gaussian() {
    let on = known_time_log_likelihood(1_700_000_000.0, 1_700_000_000.0, 2.0).expect("ok");
    let off = known_time_log_likelihood(1_700_000_010.0, 1_700_000_000.0, 2.0).expect("ok");
    // 5σ off -> drop of 12.5, up to fp rounding.
    assert!((on - off - 12.5).abs() < 1e-9);
    assert!(matches!(
        known_time_log_likelihood(0.0, 0.0, 0.0),
        Err(SearchError::NotFinite)
    ));
}

#[test]
fn negative_fit_variance_is_rejected() {
    let fit = fit(12.5, -1.0);
    assert!(matches!(
        recording_start_sigma(&fit, 1.0),
        Err(SearchError::NotFinite)
    ));
}
