//! Cramer-Rao lower bound report (FR-SIM-03).

use core_types::Metres;

/// The bound on position accuracy for one simulated scenario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrlbReport {
    /// Square root of the position covariance's largest eigenvalue, metres.
    pub position_sigma_m: Metres,
    /// Number of events that contributed information.
    pub event_count: usize,
}
