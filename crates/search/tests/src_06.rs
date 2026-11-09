//! SRC-06 / SDD 6.3 stage 2: pair join on the `(cell, clock bin)` spatial
//! hash — only cells shared by two events survive.

#![allow(clippy::expect_used)]

use core_types::{LatLon, UtcMillis};
use search::{
    Hypothesis, JoinConfig, PROVISIONAL_CLOCK_BIN_S, Prediction, SearchError, WorkBudget,
    join_pairs,
};

fn prediction(heard_ms: i64) -> Prediction {
    Prediction {
        emission_utc: UtcMillis(heard_ms - 5_000),
        heard_utc: UtcMillis(heard_ms),
        slant_range_m: 2_000.0,
        speed_m_per_s: 40.0,
        observer_height_m: 100.0,
    }
}

fn hypothesis(
    event_index: usize,
    track_index: usize,
    lat: f64,
    lon: f64,
    implied_start_s: f64,
    sigma_s: f64,
) -> Hypothesis {
    Hypothesis {
        event_index,
        track_index,
        observer: LatLon::new(lat, lon),
        prediction: prediction(1_700_000_000_000 + (implied_start_s * 1_000.0) as i64),
        implied_start_unix_s: implied_start_s,
        clock_sigma_s: sigma_s,
        maha_distance: 1.0,
    }
}

fn config() -> JoinConfig {
    JoinConfig {
        cell_size_m: 500.0,
        reference_lat_deg: 52.0,
        clock_bin_s: PROVISIONAL_CLOCK_BIN_S,
    }
}

#[test]
fn two_events_sharing_cell_and_bin_survive() {
    // Both events imply the same cell and starts 1 s apart — inside one 4 s
    // bin only if they don't straddle an edge; 100.0 and 101.0 share bin 25.
    let hyps = vec![
        hypothesis(0, 0, 52.001, 13.001, 100.0, 1.0),
        hypothesis(1, 1, 52.001, 13.001, 101.0, 2.0),
    ];
    let candidates = join_pairs(&hyps, &config(), &WorkBudget::default())
        .expect("join succeeds")
        .candidates;
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.event_indices.len(), 2);
    assert_eq!(candidate.hypothesis_indices.len(), 2);
    // Inverse-variance weighted mean of 100 (σ=1) and 101 (σ=2):
    // (100·1 + 101·0.25) / 1.25 = 100.2.
    assert!((candidate.implied_start_unix_s - 100.2).abs() < 1e-9);
    assert!((candidate.clock_sigma_s - 1.0 / 1.25_f64.sqrt()).abs() < 1e-9);
}

#[test]
fn different_clock_bins_do_not_join() {
    // Starts 10 s apart fall in different 4 s bins.
    let hyps = vec![
        hypothesis(0, 0, 52.001, 13.001, 100.0, 1.0),
        hypothesis(1, 1, 52.001, 13.001, 110.0, 1.0),
    ];
    assert!(
        join_pairs(&hyps, &config(), &WorkBudget::default())
            .expect("ok")
            .candidates
            .is_empty()
    );
}

#[test]
fn different_cells_do_not_join() {
    // ~5 km apart in latitude — different cells at 500 m.
    let hyps = vec![
        hypothesis(0, 0, 52.001, 13.001, 100.0, 1.0),
        hypothesis(1, 1, 52.05, 13.001, 100.0, 1.0),
    ];
    assert!(
        join_pairs(&hyps, &config(), &WorkBudget::default())
            .expect("ok")
            .candidates
            .is_empty()
    );
}

#[test]
fn single_event_cannot_form_a_candidate() {
    // Two hypotheses but only one event index.
    let hyps = vec![
        hypothesis(0, 0, 52.001, 13.001, 100.0, 1.0),
        hypothesis(0, 1, 52.001, 13.001, 101.0, 1.0),
    ];
    assert!(
        join_pairs(&hyps, &config(), &WorkBudget::default())
            .expect("ok")
            .candidates
            .is_empty()
    );
}

#[test]
fn same_event_does_not_double_count_in_clock_mean() {
    // Event 0 has two hypotheses in the group (different tracks); the clock
    // mean must use only the best (lowest maha) one.
    let mut weak = hypothesis(0, 0, 52.001, 13.001, 100.0, 1.0);
    weak.maha_distance = 3.0;
    let mut strong = hypothesis(0, 1, 52.001, 13.001, 100.0, 1.0);
    strong.maha_distance = 0.5;
    let other = hypothesis(1, 2, 52.001, 13.001, 103.0, 1.0);
    let hyps = vec![weak, strong, other];
    let candidates = join_pairs(&hyps, &config(), &WorkBudget::default())
        .expect("ok")
        .candidates;
    assert_eq!(candidates.len(), 1);
    // Equal sigmas: mean of 100 and 103 = 101.5.
    assert!((candidates[0].implied_start_unix_s - 101.5).abs() < 1e-9);
}

#[test]
fn invalid_config_and_hypotheses_are_rejected() {
    let hyps = vec![hypothesis(0, 0, 52.0, 13.0, 100.0, 1.0)];
    let bad = JoinConfig {
        clock_bin_s: 0.0,
        ..config()
    };
    assert!(matches!(
        join_pairs(&hyps, &bad, &WorkBudget::default()),
        Err(SearchError::NotFinite)
    ));
    let mut bad_hyp = hypothesis(0, 0, 52.0, 13.0, f64::NAN, 1.0);
    bad_hyp.prediction.heard_utc = UtcMillis(0);
    assert!(matches!(
        join_pairs(&[bad_hyp], &config(), &WorkBudget::default()),
        Err(SearchError::NotFinite)
    ));
}
