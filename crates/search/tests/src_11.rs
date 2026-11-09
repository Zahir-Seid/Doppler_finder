//! SRC-11 / FR-SL-05: ranked cluster alternatives with posterior mass and
//! the margin over the best rival.

#![allow(clippy::expect_used)]

use search::{CandidateCluster, ranked_alternatives};

fn cluster(log_mass: f64, posterior_mass: f64) -> CandidateCluster {
    CandidateCluster {
        member_indices: vec![0],
        log_mass,
        posterior_mass,
        best_log_posterior: log_mass,
    }
}

#[test]
fn ranked_alternatives_reports_mass_and_margin() {
    // Three clusters: leader at log_mass 0, rivals 4 and 9 nats behind.
    let clusters = vec![cluster(0.0, 0.9), cluster(-4.0, 0.09), cluster(-9.0, 0.01)];
    let ranked = ranked_alternatives(&clusters);
    assert_eq!(ranked.len(), 3);

    // Leader: margin over best rival (#2 at -4) is +4 nats.
    assert_eq!(ranked[0].rank, 0);
    assert!((ranked[0].margin_nats - 4.0).abs() < 1e-12);
    assert_eq!(ranked[0].cluster.posterior_mass, 0.9);

    // First rival trails the leader by 4 nats.
    assert_eq!(ranked[1].rank, 1);
    assert!((ranked[1].margin_nats - (-4.0)).abs() < 1e-12);

    // Second rival's best rival is still the leader: -9 nats.
    assert_eq!(ranked[2].rank, 2);
    assert!((ranked[2].margin_nats - (-9.0)).abs() < 1e-12);
}

#[test]
fn single_cluster_has_infinite_margin() {
    let clusters = vec![cluster(0.0, 1.0)];
    let ranked = ranked_alternatives(&clusters);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].margin_nats, f64::INFINITY);
}

#[test]
fn empty_input_gives_empty_output() {
    assert_eq!(ranked_alternatives(&[]), Vec::new());
}

#[test]
fn margin_uses_log_mass_not_normalized_mass() {
    // Normalization constant must cancel: shifting all log_mass by the
    // same amount leaves every margin unchanged.
    let a = vec![cluster(10.0, 0.8), cluster(6.0, 0.2)];
    let b = vec![cluster(-3.0, 0.8), cluster(-7.0, 0.2)];
    let ra = ranked_alternatives(&a);
    let rb = ranked_alternatives(&b);
    assert_eq!(ra[0].margin_nats, rb[0].margin_nats);
    assert_eq!(ra[1].margin_nats, rb[1].margin_nats);
}
