//! Continuous position estimate and covariance ellipse (FR-SL-03).

use core_types::Metres;

/// A two-dimensional position estimate in a local ENU frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalizationEstimate {
    /// East coordinate in metres.
    pub east_m: Metres,
    /// North coordinate in metres.
    pub north_m: Metres,
    /// Symmetric 2x2 covariance in square metres.
    pub covariance_m2: [[f64; 2]; 2],
    /// Posterior mass assigned to this estimate's cluster, in `[0, 1]`.
    pub posterior_mass: f64,
}

impl LocalizationEstimate {
    /// Semi-major axis of the 1-sigma covariance ellipse in metres.
    pub fn semi_major_m(&self) -> Metres {
        let [[var_east, cov_en], [_cov_ne, var_north]] = self.covariance_m2;
        let half_trace = f64::midpoint(var_east, var_north);
        let determinant = var_east * var_north - cov_en * cov_en;
        let root = (half_trace * half_trace - determinant).max(0.0).sqrt();
        Metres(half_trace + root)
    }

    /// Semi-minor axis of the 1-sigma covariance ellipse in metres.
    pub fn semi_minor_m(&self) -> Metres {
        let [[var_east, cov_en], [_cov_ne, var_north]] = self.covariance_m2;
        let half_trace = f64::midpoint(var_east, var_north);
        let determinant = var_east * var_north - cov_en * cov_en;
        let root = (half_trace * half_trace - determinant).max(0.0).sqrt();
        Metres((half_trace - root).max(0.0))
    }
}
