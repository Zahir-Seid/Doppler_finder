//! SRC-04: silence term — Poisson-binomial score over unexplained audible
//! passes, jet exclusion and probabilistic unknown classes.
#![allow(clippy::expect_used)]

use std::collections::BTreeSet;

use adsb::EngineClass;
use search::{DetectionModel, SilencePass, silence_log_likelihood};

fn pass(leg_id: &str, class: EngineClass, range_m: f64, heard_unix_s: f64) -> SilencePass {
    SilencePass {
        leg_id: leg_id.to_owned(),
        engine_class: class,
        slant_range_m: range_m,
        heard_unix_s,
    }
}

fn explained(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn close_unexplained_propeller_pass_costs_likelihood() {
    // At range 0 the logistic gives p = 1/(1+e^-3.33) ≈ 0.965.
    let passes = [pass("leg:a", EngineClass::Propeller, 0.0, 100.0)];
    let model = DetectionModel::default();
    let ll = silence_log_likelihood(&passes, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    let p = model.p_detect(&passes[0]).expect("ok");
    assert!((ll - (1.0 - p).ln()).abs() < 1e-12);
    assert!(ll < -2.0, "close unexplained pass should cost several nats");
}

#[test]
fn far_pass_contributes_nearly_nothing() {
    let near = [pass("leg:a", EngineClass::Propeller, 0.0, 100.0)];
    let far = [pass("leg:a", EngineClass::Propeller, 1.0e9, 100.0)];
    let model = DetectionModel::default();
    let ll_near = silence_log_likelihood(&near, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    let ll_far = silence_log_likelihood(&far, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    assert!(ll_far > ll_near);
    assert!(ll_far > -1e-6, "a 1000 km pass is not a missed detection");
}

#[test]
fn jets_are_excluded_outright() {
    let passes = [pass("leg:j", EngineClass::Jet, 100.0, 100.0)];
    let model = DetectionModel::default();
    assert_eq!(model.p_detect(&passes[0]).expect("ok"), 0.0);
    let ll = silence_log_likelihood(&passes, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    assert_eq!(ll, 0.0);
}

#[test]
fn unknown_class_is_prior_scaled() {
    let prop = pass("leg:p", EngineClass::Propeller, 5_000.0, 100.0);
    let unk = pass("leg:u", EngineClass::Unknown, 5_000.0, 100.0);
    let model = DetectionModel::default();
    let p_prop = model.p_detect(&prop).expect("ok");
    let p_unk = model.p_detect(&unk).expect("ok");
    assert!((p_unk - 0.5 * p_prop).abs() < 1e-12);
}

#[test]
fn explained_passes_and_out_of_window_passes_do_not_count() {
    let passes = [
        pass("leg:explained", EngineClass::Propeller, 0.0, 100.0),
        pass("leg:before", EngineClass::Propeller, 0.0, -50.0),
        pass("leg:after", EngineClass::Propeller, 0.0, 250.0),
        pass("leg:missed", EngineClass::Propeller, 0.0, 150.0),
    ];
    let model = DetectionModel::default();
    let ll = silence_log_likelihood(&passes, &explained(&["leg:explained"]), 0.0, 200.0, &model)
        .expect("ok");
    let only_missed = [passes[3].clone()];
    let ll_only =
        silence_log_likelihood(&only_missed, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    assert!((ll - ll_only).abs() < 1e-12);
}

#[test]
fn two_passes_give_poisson_binomial_product() {
    let passes = [
        pass("leg:a", EngineClass::Propeller, 2_000.0, 100.0),
        pass("leg:b", EngineClass::Propeller, 8_000.0, 150.0),
    ];
    let model = DetectionModel::default();
    let ll = silence_log_likelihood(&passes, &BTreeSet::new(), 0.0, 200.0, &model).expect("ok");
    let expected: f64 = passes
        .iter()
        .map(|p| (1.0 - model.p_detect(p).expect("ok")).ln())
        .sum();
    assert!((ll - expected).abs() < 1e-12);
}

#[test]
fn invalid_inputs_are_rejected() {
    let model = DetectionModel::default();
    let bad_range = [pass("leg:a", EngineClass::Propeller, f64::NAN, 100.0)];
    assert!(silence_log_likelihood(&bad_range, &BTreeSet::new(), 0.0, 200.0, &model).is_err());
    let bad_time = [pass("leg:a", EngineClass::Propeller, 1.0, f64::INFINITY)];
    assert!(silence_log_likelihood(&bad_time, &BTreeSet::new(), 0.0, 200.0, &model).is_err());
    assert!(silence_log_likelihood(&[], &BTreeSet::new(), 200.0, 100.0, &model).is_err());
    let bad_model = DetectionModel {
        width_m: 0.0,
        ..DetectionModel::default()
    };
    let good = [pass("leg:a", EngineClass::Propeller, 1.0, 100.0)];
    assert!(silence_log_likelihood(&good, &BTreeSet::new(), 0.0, 200.0, &bad_model).is_err());
}
