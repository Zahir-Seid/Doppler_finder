//! SRC-09 / SDD 6.4, FR-SL-04, NFR-01: the calibration factor `κ` — fitted
//! on simulator `d²` samples, stored with the model version in every
//! result manifest.

#![allow(clippy::expect_used)]

use core_types::manifest::ModelVersions;
use search::{Calibration, PROVISIONAL_KAPPA, fit_kappa};

/// Deterministic χ²₂ samples: `d² = -2 ln(1 - u)` for evenly spaced `u`,
/// i.e. exact chi-square-2 quantiles. A perfectly calibrated ellipse
/// yields `κ = 1`.
fn chi2_samples(n: usize, scale: f64) -> Vec<f64> {
    (1..=n)
        .map(|i| {
            let u = i as f64 / (n as f64 + 1.0);
            scale * -2.0 * (1.0 - u).ln()
        })
        .collect()
}

#[test]
fn fit_kappa_returns_one_for_calibrated_samples() {
    let d2 = chi2_samples(200, 1.0);
    let kappa = fit_kappa(&d2, 0.95).expect("fit succeeds");
    assert!(
        (kappa - 1.0).abs() < 0.05,
        "expected κ≈1 for χ²₂ samples, got {kappa}"
    );
}

#[test]
fn fit_kappa_scales_with_error_magnitude() {
    // Errors twice as large as the ellipse claims → κ ≈ 2.
    let d2 = chi2_samples(200, 4.0);
    let kappa = fit_kappa(&d2, 0.95).expect("fit succeeds");
    assert!(
        (kappa - 2.0).abs() < 0.1,
        "expected κ≈2 for 2× overdispersed samples, got {kappa}"
    );
}

#[test]
fn fit_kappa_rejects_invalid_inputs() {
    assert!(fit_kappa(&[], 0.95).is_err());
    assert!(fit_kappa(&[1.0, f64::NAN], 0.95).is_err());
    assert!(fit_kappa(&[1.0, -1.0], 0.95).is_err());
    assert!(fit_kappa(&[1.0, 2.0], 0.0).is_err());
    assert!(fit_kappa(&[1.0, 2.0], 1.0).is_err());
    assert!(fit_kappa(&[1.0, 2.0], f64::NAN).is_err());
}

#[test]
fn calibration_scales_only_the_position_block() {
    let cal = Calibration::new(2.0, "test-v1").expect("valid calibration");
    let cov = [[4.0, 1.0, 0.5], [1.0, 9.0, 0.25], [0.5, 0.25, 0.04]];
    let ellipse = cal.ellipse_covariance(&cov);
    // κ² = 4 scales the (e, n) block only; the s row/column is untouched.
    assert_eq!(ellipse[0][0], 16.0);
    assert_eq!(ellipse[0][1], 4.0);
    assert_eq!(ellipse[1][0], 4.0);
    assert_eq!(ellipse[1][1], 36.0);
}

#[test]
fn calibration_rejects_nonpositive_kappa() {
    assert!(Calibration::new(0.0, "v").is_err());
    assert!(Calibration::new(-1.0, "v").is_err());
    assert!(Calibration::new(f64::NAN, "v").is_err());
    assert!(Calibration::new(f64::INFINITY, "v").is_err());
}

#[test]
fn provisional_calibration_is_uncalibrated_and_versioned() {
    let cal = Calibration::provisional();
    assert_eq!(cal.kappa, PROVISIONAL_KAPPA);
    assert_eq!(cal.kappa, 1.0);
    let cov = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    assert_eq!(cal.ellipse_covariance(&cov)[0][0], 1.0);

    let versions: ModelVersions = cal.model_versions("search-v0");
    assert_eq!(versions.model, "search-v0");
    assert_eq!(versions.kappa, 1.0);
    assert_eq!(versions.calibration, cal.version);
}
