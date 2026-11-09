//! SRC-10 / SDD 6.5, FR-SL-02, FR-OUT-01: the verdict table — locatable
//! gates, ambiguity caps, refusals and reason codes.

#![allow(clippy::expect_used)]

use search::{ReasonCode, Verdict, VerdictInput, VerdictThresholds, decide, ellipse_semi_major_95};

fn input() -> VerdictInput {
    VerdictInput {
        top_mass: 0.95,
        rival_mass: 0.0,
        independent_events: 2,
        known_time: false,
        silence_support: false,
        ellipse_semi_major_95_m: 500.0,
        truncated: false,
        veto: false,
    }
}

fn thresholds() -> VerdictThresholds {
    VerdictThresholds::default()
}

#[test]
fn clean_two_event_candidate_is_locatable() {
    let decision = decide(&input(), &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::Locatable);
    assert_eq!(decision.reason_codes, Vec::<ReasonCode>::new());
}

#[test]
fn single_event_without_known_time_is_always_refused() {
    let mut i = input();
    i.independent_events = 1;
    i.known_time = false;
    i.top_mass = 0.99;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::NotLocatable);
    assert_eq!(decision.reason_codes, [ReasonCode::SingleEvent]);
}

#[test]
fn single_event_with_known_time_and_silence_can_be_locatable() {
    let mut i = input();
    i.independent_events = 1;
    i.known_time = true;
    i.silence_support = true;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::Locatable);
}

#[test]
fn single_event_with_known_time_but_no_silence_is_ambiguous() {
    let mut i = input();
    i.independent_events = 1;
    i.known_time = true;
    i.silence_support = false;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::Ambiguous);
    assert_eq!(decision.reason_codes, [ReasonCode::CurveOnly]);
}

#[test]
fn mass_below_ambiguous_bar_is_not_locatable() {
    let mut i = input();
    i.top_mass = 0.3;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::NotLocatable);
    assert_eq!(decision.reason_codes, [ReasonCode::LowPosteriorMass]);

    i.top_mass = 0.0;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.reason_codes, [ReasonCode::NoFlightMatch]);
}

#[test]
fn mid_mass_is_ambiguous() {
    let mut i = input();
    i.top_mass = 0.7;
    let decision = decide(&i, &thresholds()).expect("decide succeeds");
    assert_eq!(decision.verdict, Verdict::Ambiguous);
}

#[test]
fn each_locatable_condition_caps_at_ambiguous() {
    // Ellipse too large.
    let mut i = input();
    i.ellipse_semi_major_95_m = 1_500.0;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::Ambiguous);
    assert_eq!(d.reason_codes, [ReasonCode::EllipseTooLarge]);

    // Truncated search.
    let mut i = input();
    i.truncated = true;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::Ambiguous);
    assert_eq!(d.reason_codes, [ReasonCode::TruncatedSearch]);

    // Significant rival cluster (mirror pair).
    let mut i = input();
    i.rival_mass = 0.4;
    i.top_mass = 0.6;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::Ambiguous);
    assert!(d.reason_codes.contains(&ReasonCode::MultipleClusters));

    // Zero independent events cannot happen with mass, but the guard holds.
    let mut i = input();
    i.independent_events = 0;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::Ambiguous);
    assert_eq!(d.reason_codes, [ReasonCode::InsufficientEvents]);
}

#[test]
fn veto_is_not_locatable() {
    let mut i = input();
    i.veto = true;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::NotLocatable);
    assert_eq!(d.reason_codes, [ReasonCode::Veto]);
}

#[test]
fn multiple_failures_collect_multiple_reasons() {
    let mut i = input();
    i.truncated = true;
    i.ellipse_semi_major_95_m = 2_000.0;
    i.rival_mass = 0.2;
    i.top_mass = 0.8;
    let d = decide(&i, &thresholds()).expect("decide");
    assert_eq!(d.verdict, Verdict::Ambiguous);
    assert!(d.reason_codes.contains(&ReasonCode::TruncatedSearch));
    assert!(d.reason_codes.contains(&ReasonCode::EllipseTooLarge));
    assert!(d.reason_codes.contains(&ReasonCode::MultipleClusters));
}

#[test]
fn decide_rejects_bad_thresholds_and_inputs() {
    let mut bad = thresholds();
    bad.ambiguous_min = 0.95; // > locatable_min? no: 0.95 > 0.9 → invalid order
    bad.locatable_min = 0.9;
    assert!(decide(&input(), &bad).is_err());

    let mut i = input();
    i.top_mass = f64::NAN;
    assert!(decide(&i, &thresholds()).is_err());
    i.top_mass = 1.5;
    assert!(decide(&i, &thresholds()).is_err());
}

#[test]
fn semi_major_axis_matches_hand_computed_value() {
    // Isotropic ellipse: cov = σ²I → λmax = σ² → semi = σ·sqrt(χ²₂(0.95)).
    let ellipse = [[100.0, 0.0], [0.0, 100.0]];
    let semi = ellipse_semi_major_95(&ellipse);
    let expected = 10.0 * (-2.0 * 0.05_f64.ln()).sqrt();
    assert!((semi - expected).abs() < 1e-9);

    // Degenerate input → INFINITY so the verdict cap triggers.
    let bad = [[f64::NAN, 0.0], [0.0, 1.0]];
    assert_eq!(ellipse_semi_major_95(&bad), f64::INFINITY);
}
