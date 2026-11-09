//! SRC-07 / SDD 6.3 stages 3–4: full log-posterior over join survivors,
//! log-sum-exp over assignments, top-K clusters with normalized masses,
//! and the land prior hook.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;

use core_types::{AudioSeconds, EventId, Hertz, LatLon, Metres, UtcMillis};
use events::EventFit;
use search::{
    ErrorBudget, Hypothesis, JoinCandidate, LandPrior, PosteriorEnv, Prediction, SearchError,
    UniformPrior, score_candidates, top_clusters,
};

fn prediction(speed: f64, closest: f64, heard_ms: i64) -> Prediction {
    Prediction {
        emission_utc: UtcMillis(heard_ms - 5_000),
        heard_utc: UtcMillis(heard_ms),
        slant_range_m: closest,
        speed_m_per_s: speed,
        observer_height_m: 100.0,
    }
}

fn fit(speed: f64, closest: f64, heard_at_s: f64, heard_var: f64) -> EventFit {
    let mut covariance = [[0.0; 4]; 4];
    covariance[1][1] = 1.0;
    covariance[2][2] = 100.0;
    covariance[3][3] = heard_var;
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s: speed,
        closest_m: Metres(closest),
        heard_at: AudioSeconds(heard_at_s),
        covariance,
    }
}

/// Hypothesis whose prediction matches `fit` exactly: speed `v`, slant
/// range `d`, heard 10 s after the implied start.
fn hyp(
    event_index: usize,
    track_index: usize,
    lat: f64,
    lon: f64,
    implied_start_s: f64,
    speed: f64,
    closest: f64,
) -> Hypothesis {
    let heard_ms = ((implied_start_s + 10.0) * 1_000.0) as i64;
    Hypothesis {
        event_index,
        track_index,
        observer: LatLon::new(lat, lon),
        prediction: prediction(speed, closest, heard_ms),
        implied_start_unix_s: implied_start_s,
        clock_sigma_s: 1.0,
        maha_distance: 0.5,
    }
}

fn candidate(indices: Vec<usize>, events: &[usize], lat: f64, lon: f64, s: f64) -> JoinCandidate {
    JoinCandidate {
        hypothesis_indices: indices,
        event_indices: events.iter().copied().collect::<BTreeSet<_>>(),
        cell_lat_deg: lat,
        cell_lon_deg: lon,
        implied_start_unix_s: s,
        clock_sigma_s: 0.7,
    }
}

fn env<'a>(budget: &'a ErrorBudget, silence: &'a [Option<f64>]) -> PosteriorEnv<'a> {
    PosteriorEnv {
        budget,
        adsb_timing_sigma_s: 0.5,
        known_time: None,
        silence,
        prior: &UniformPrior,
    }
}

#[test]
fn consistent_candidate_outranks_mismatched_one() {
    // Events fit (40 m/s, 2000 m); candidate A's predictions match, B's do
    // not — A must score higher.
    let events = [
        fit(40.0, 2_000.0, 10.0, 0.25),
        fit(40.0, 2_000.0, 10.0, 0.25),
    ];
    let hyps = vec![
        hyp(0, 0, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0),
        hyp(1, 1, 52.0, 13.0, 1_700_000_001.0, 40.0, 2_000.0),
        hyp(0, 2, 52.01, 13.01, 1_700_001_000.0, 200.0, 9_000.0),
        hyp(1, 3, 52.01, 13.01, 1_700_001_001.0, 200.0, 9_000.0),
    ];
    let candidates = vec![
        candidate(vec![0, 1], &[0, 1], 52.0, 13.0, 1_700_000_000.5),
        candidate(vec![2, 3], &[0, 1], 52.01, 13.01, 1_700_001_000.5),
    ];
    let budget = ErrorBudget::default();
    let silence = [None, None];
    let scored = score_candidates(&candidates, &hyps, &events, &env(&budget, &silence))
        .expect("scoring succeeds");
    assert_eq!(scored.len(), 2);
    assert_eq!(
        scored[0].candidate_index, 0,
        "consistent candidate must rank first"
    );
    assert!(scored[0].log_posterior > scored[1].log_posterior);
}

#[test]
fn two_assignments_double_the_mass() {
    // Event 0 has two track options with identical predictions: two equal
    // assignments, so the candidate score is single + ln 2.
    let events = [fit(40.0, 2_000.0, 10.0, 0.25)];
    let mut hyps = vec![
        hyp(0, 0, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0),
        hyp(0, 1, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0),
    ];
    // Give the second hypothesis a different track_index so it is a
    // distinct assignment; identical otherwise.
    hyps[1].track_index = 7;
    let candidates = vec![candidate(vec![0, 1], &[0], 52.0, 13.0, 1_700_000_000.0)];
    // A single-option candidate for the baseline.
    let single = vec![candidate(vec![0], &[0], 52.0, 13.0, 1_700_000_000.0)];
    let budget = ErrorBudget::default();
    let silence = [None];
    let env = env(&budget, &silence);
    let scored_two = score_candidates(&candidates, &hyps, &events, &env).expect("ok");
    let scored_one = score_candidates(&single, &hyps, &events, &env).expect("ok");
    let diff = scored_two[0].log_posterior - scored_one[0].log_posterior;
    assert!((diff - std::f64::consts::LN_2).abs() < 1e-9, "diff {diff}");
    assert_eq!(scored_two[0].assignments_evaluated, 2);
}

#[test]
fn land_prior_can_exclude_a_candidate() {
    struct OceanPrior;
    impl LandPrior for OceanPrior {
        fn log_prior(&self, observer: LatLon) -> f64 {
            if observer.lat_deg.0 > 52.005 {
                f64::NEG_INFINITY
            } else {
                0.0
            }
        }
    }
    let events = [fit(40.0, 2_000.0, 10.0, 0.25)];
    let hyps = vec![hyp(0, 0, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0)];
    let candidates = vec![candidate(vec![0], &[0], 52.01, 13.0, 1_700_000_000.0)];
    let budget = ErrorBudget::default();
    let silence = [None];
    let mut e = env(&budget, &silence);
    e.prior = &OceanPrior;
    let scored = score_candidates(&candidates, &hyps, &events, &e).expect("ok");
    assert_eq!(scored[0].log_posterior, f64::NEG_INFINITY);
}

#[test]
fn top_clusters_merges_nearby_and_normalizes() {
    // Three candidates: two ~200 m apart, one ~5 km away. All score decently.
    let events = [fit(40.0, 2_000.0, 10.0, 0.25)];
    let hyps: Vec<Hypothesis> = (0..3)
        .map(|i| hyp(0, i, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0))
        .collect();
    let candidates = vec![
        candidate(vec![0], &[0], 52.0, 13.0, 1_700_000_000.0),
        candidate(vec![1], &[0], 52.0018, 13.0, 1_700_000_000.0), // ~200 m north
        candidate(vec![2], &[0], 52.05, 13.0, 1_700_000_000.0),   // ~5.5 km north
    ];
    let budget = ErrorBudget::default();
    let silence = [None, None, None];
    let scored =
        score_candidates(&candidates, &hyps, &events, &env(&budget, &silence)).expect("ok");
    let clusters = top_clusters(&scored, &candidates, 8, 1_000.0).expect("ok");
    assert_eq!(clusters.len(), 2, "near pair merges, far one separate");
    let mass_sum: f64 = clusters.iter().map(|c| c.posterior_mass).sum();
    assert!(
        (mass_sum - 1.0).abs() < 1e-9,
        "masses must normalize: {mass_sum}"
    );
    // The merged pair holds more mass than the singleton.
    assert!(clusters[0].posterior_mass > clusters[1].posterior_mass);
    assert_eq!(clusters[0].member_indices.len(), 2);
}

#[test]
fn invalid_inputs_are_rejected() {
    let events = [fit(40.0, 2_000.0, 10.0, 0.25)];
    let hyps = vec![hyp(0, 0, 52.0, 13.0, 1_700_000_000.0, 40.0, 2_000.0)];
    let candidates = vec![candidate(vec![0], &[0], 52.0, 13.0, 1_700_000_000.0)];
    let budget = ErrorBudget::default();
    // silence slice length mismatch
    let bad_silence: [Option<f64>; 0] = [];
    assert!(matches!(
        score_candidates(&candidates, &hyps, &events, &env(&budget, &bad_silence)),
        Err(SearchError::NotFinite)
    ));
    assert!(matches!(
        top_clusters(&[], &candidates, 0, 1_000.0),
        Err(SearchError::NotFinite)
    ));
}
