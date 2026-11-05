//! Per-event observations and fitted parameters (FR-DM-02 to FR-DM-05).

use core_types::{AudioSeconds, EventId, Hertz, Metres};

use crate::error::EventError;

/// One detected harmonic ridge, before fitting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventObservation {
    /// Heard closest-approach time on the audio clock.
    pub heard_at: AudioSeconds,
    /// Observed fundamental frequency in hertz.
    pub fundamental_hz: Hertz,
    /// Signal-to-noise ratio along the ridge in decibels.
    pub snr_db: f64,
    /// Share of the ridge samples that were usable, in `[0, 1]`.
    pub coverage: f64,
}

impl EventObservation {
    /// Check that the observation is physically and numerically valid.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.heard_at.0.is_finite()
            || !self.fundamental_hz.0.is_finite()
            || !self.snr_db.is_finite()
            || !self.coverage.is_finite()
        {
            return Err(EventError::NotFinite);
        }
        if !(0.0..=1.0).contains(&self.coverage) {
            return Err(EventError::QualityOutOfRange(self.coverage));
        }
        Ok(())
    }
}

/// A detected event with its harmonic assignment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    /// Stable event identifier.
    pub id: EventId,
    /// Harmonic index, with 1 the fundamental.
    pub harmonic: u32,
    /// Ridge observation.
    pub observation: EventObservation,
}

impl Event {
    /// Check the event and its observation.
    pub fn validate(&self) -> Result<(), EventError> {
        if self.harmonic == 0 {
            return Err(EventError::InvalidHarmonic(self.harmonic));
        }
        self.observation.validate()
    }
}

/// Fitted parameters of one event, with their covariance.
///
/// The parameter vector is `(fundamental_hz, speed_m_per_s, closest_m,
/// heard_time_s, recording_start_s)` in that order. The covariance is the
/// inverse Hessian from the fit (SDD section 5, step 5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventFit {
    /// Event this fit belongs to.
    pub event_id: EventId,
    /// Fitted fundamental frequency.
    pub fundamental_hz: Hertz,
    /// Fitted aircraft speed.
    pub speed_m_per_s: f64,
    /// Fitted closest-approach distance.
    pub closest_m: Metres,
    /// Fitted heard closest-approach time on the audio clock.
    pub heard_at: AudioSeconds,
    /// Symmetric 5x5 parameter covariance.
    pub covariance: [[f64; 5]; 5],
}
