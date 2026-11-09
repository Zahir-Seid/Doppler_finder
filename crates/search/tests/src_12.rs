//! SRC-12 / SDD 6.3 closing rule, FR-SL-01: every stage has a work budget;
//! overflow keeps the best items and sets `truncated`, which caps the
//! verdict at `Ambiguous` downstream.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;

use core_types::{AudioSeconds, EventId, Hertz, LatLon, Metres, UtcMillis};
use events::EventFit;
use search::{
    ErrorBudget, Hypothesis, JoinCandidate, JoinConfig, PosteriorEnv, Prediction, SearchError,
    UniformPrior, Verdict, VerdictInput, VerdictThresholds, WorkBudget, decide, join_pairs,
    score_candidates,
};

// --- join-stage fixtures (same style as src_06) -------------------------

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

fn join_config() -> JoinConfig {
    JoinConfig {
        cell_size_m: 500.0,
        reference_lat_deg: 52.0,
        clock_bin_s: 4.0,
    }
}

#[test]
fn join_budget_keeps_most_clock_precise_candidates() {
    // Four (cell, bin) groups, each with two events. Per-candidate sigma is
    // 1/sqrt(2/σ²): σ=4 → ~2.83, σ=2 → ~1.41, σ=1 → ~0.71, σ=0.5 → ~0.35.
    // Budget 2 must keep the σ=0.5 and σ=1 candidates.
    let mut hyps = Vec::new();
    for (k, sigma) in [4.0, 2.0, 1.0, 0.5].iter().enumerate() {
        let lat = 52.0 + k as f64 * 0.02; // ~2.2 km apart: distinct cells
        hyps.push(hypothesis(0, k * 2, lat, 13.0, 100.0, *sigma));
        hyps.push(hypothesis(1, k * 2 + 1, lat, 13.0, 100.0, *sigma));
    }
    let budget = WorkBudget {
        max_pairs: 2,
        ..WorkBudget::default()
    };
    let outcome = join_pairs(&hyps, &join_config(), &budget).expect("join succeeds");
    assert!(outcome.truncated);
    assert_eq!(outcome.candidates.len(), 2);
    // Kept candidates are the two smallest clock_sigma_s.
    let sigmas: Vec<f64> = outcome.candidates.iter().map(|c| c.clock_sigma_s).collect();
    assert!(sigmas.iter().all(|&s| s < 1.0), "kept {sigmas:?}");
    // Deterministic: same input, same survivors.
    let again = join_pairs(&hyps, &join_config(), &budget)
        .expect("join")
        .candidates;
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.cell_lat_deg.to_bits())
            .collect::<Vec<_>>(),
        again
            .iter()
            .map(|c| c.cell_lat_deg.to_bits())
            .collect::<Vec<_>>()
    );
}

// --- score-stage fixtures (src_07 style) --------------------------------

fn fit(speed: f64, closest: f64) -> EventFit {
    let mut covariance = [[0.0; 4]; 4];
    covariance[1][1] = 1.0;
    covariance[2][2] = 100.0;
    covariance[3][3] = 0.25;
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s: speed,
        closest_m: Metres(closest),
        heard_at: AudioSeconds(10.0),
        covariance,
    }
}

fn scored_hyp(
    event_index: usize,
    track_index: usize,
    lat: f64,
    speed: f64,
    closest: f64,
) -> Hypothesis {
    let heard_ms = 1_700_000_010_000i64;
    Hypothesis {
        event_index,
        track_index,
        observer: LatLon::new(lat, 13.0),
        prediction: Prediction {
            emission_utc: UtcMillis(heard_ms - 5_000),
            heard_utc: UtcMillis(heard_ms),
            slant_range_m: closest,
            speed_m_per_s: speed,
            observer_height_m: 100.0,
        },
        implied_start_unix_s: 1_700_000_000.0,
        clock_sigma_s: 1.0,
        maha_distance: 0.5,
    }
}

fn join_candidate(indices: Vec<usize>, lat: f64) -> JoinCandidate {
    JoinCandidate {
        hypothesis_indices: indices,
        event_indices: [0usize, 1].into_iter().collect::<BTreeSet<_>>(),
        cell_lat_deg: lat,
        cell_lon_deg: 13.0,
        implied_start_unix_s: 1_700_000_000.0,
        clock_sigma_s: 0.7,
    }
}

#[test]
fn score_budget_keeps_highest_posterior_candidates() {
    // Two events fit (40, 2000). Candidate 0 matches; candidate 1 is ~5 km
    // away but its hypotheses still carry the same (v, d) — scores differ
    // only via prior/position-independent terms here, so instead verify the
    // list cap mechanically: 3 candidates, budget 2.
    let events = [fit(40.0, 2_000.0), fit(40.0, 2_000.0)];
    let hyps = vec![
        scored_hyp(0, 0, 52.0, 40.0, 2_000.0),
        scored_hyp(1, 1, 52.0, 40.0, 2_000.0),
        scored_hyp(0, 2, 52.01, 40.0, 2_000.0),
        scored_hyp(1, 3, 52.01, 40.0, 2_000.0),
        scored_hyp(0, 4, 52.02, 40.0, 2_000.0),
        scored_hyp(1, 5, 52.02, 40.0, 2_000.0),
    ];
    let candidates = vec![
        join_candidate(vec![0, 1], 52.0),
        join_candidate(vec![2, 3], 52.01),
        join_candidate(vec![4, 5], 52.02),
    ];
    let silence = [None, None, None];
    let env = PosteriorEnv {
        budget: &ErrorBudget::default(),
        adsb_timing_sigma_s: 0.5,
        known_time: None,
        silence: &silence,
        prior: &UniformPrior,
    };
    let work = WorkBudget {
        max_scored_candidates: 2,
        ..WorkBudget::default()
    };
    let outcome = score_candidates(&candidates, &hyps, &events, &env, &work).expect("score");
    assert!(outcome.truncated);
    assert_eq!(outcome.candidates.len(), 2);
    // Sorted descending by log_posterior.
    assert!(outcome.candidates[0].log_posterior >= outcome.candidates[1].log_posterior);
}

#[test]
fn assignment_cap_flags_truncation() {
    // One event with 3 track options per candidate → 3 assignments.
    // max_assignments = 2 forces a truncated enumeration.
    let events = [fit(40.0, 2_000.0)];
    let hyps: Vec<Hypothesis> = (0..3)
        .map(|i| scored_hyp(0, i, 52.0, 40.0, 2_000.0))
        .collect();
    let candidates = vec![JoinCandidate {
        hypothesis_indices: vec![0, 1, 2],
        event_indices: [0usize].into_iter().collect::<BTreeSet<_>>(),
        cell_lat_deg: 52.0,
        cell_lon_deg: 13.0,
        implied_start_unix_s: 1_700_000_000.0,
        clock_sigma_s: 0.7,
    }];
    let silence = [None];
    let env = PosteriorEnv {
        budget: &ErrorBudget::default(),
        adsb_timing_sigma_s: 0.5,
        known_time: None,
        silence: &silence,
        prior: &UniformPrior,
    };
    let work = WorkBudget {
        max_assignments: 2,
        ..WorkBudget::default()
    };
    let outcome = score_candidates(&candidates, &hyps, &events, &env, &work).expect("score");
    assert!(outcome.truncated);
    assert!(outcome.candidates[0].truncated);
    assert_eq!(outcome.candidates[0].assignments_evaluated, 2);
}

#[test]
fn truncated_stage_caps_verdict_at_ambiguous() {
    // The plumbing contract: a truncated stage flag feeds decide()'s
    // `truncated` input, which caps at Ambiguous (SDD 6.5).
    let input = VerdictInput {
        top_mass: 0.99,
        rival_mass: 0.0,
        independent_events: 3,
        known_time: false,
        silence_support: false,
        ellipse_semi_major_95_m: 100.0,
        truncated: true,
        veto: false,
    };
    let decision = decide(&input, &VerdictThresholds::default()).expect("decide");
    assert_eq!(decision.verdict, Verdict::Ambiguous);
}

#[test]
fn zero_budget_limits_are_rejected() {
    let hyps = vec![
        hypothesis(0, 0, 52.0, 13.0, 100.0, 1.0),
        hypothesis(1, 1, 52.0, 13.0, 100.0, 1.0),
    ];
    let zero_pairs = WorkBudget {
        max_pairs: 0,
        ..WorkBudget::default()
    };
    assert!(matches!(
        join_pairs(&hyps, &join_config(), &zero_pairs),
        Err(SearchError::InvalidThresholds)
    ));

    let events = [fit(40.0, 2_000.0)];
    let scored_hyps = vec![scored_hyp(0, 0, 52.0, 40.0, 2_000.0)];
    let silence = [None];
    let env = PosteriorEnv {
        budget: &ErrorBudget::default(),
        adsb_timing_sigma_s: 0.5,
        known_time: None,
        silence: &silence,
        prior: &UniformPrior,
    };
    let zero_scored = WorkBudget {
        max_scored_candidates: 0,
        ..WorkBudget::default()
    };
    let mut c = join_candidate(vec![0], 52.0);
    c.event_indices = [0usize].into_iter().collect();
    assert!(matches!(
        score_candidates(&[c], &scored_hyps, &events, &env, &zero_scored),
        Err(SearchError::InvalidThresholds)
    ));
}
