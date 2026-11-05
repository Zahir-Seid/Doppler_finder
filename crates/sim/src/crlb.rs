//! Cramér–Rao lower bound on event parameters and on position (FR-SIM-03).
//!
//! SDD section 7 asks for the bound that says whether an estimator is limited
//! by noise or by the model. The bound is built in two steps.
//!
//! 1. **Event parameters.** The measurement is the observed frequency curve of
//!    SDD section 4, `f(t) = f0 c / (c + v_r(t_e))`, sampled at the audio rate
//!    with independent Gaussian frequency noise of standard deviation
//!    `sigma_hz`. The Fisher information of the four parameters
//!    `(f0, v, d, t0)` is `J_ij = (1/sigma²) Σ_k (∂f/∂θ_i)(∂f/∂θ_j)`, and its
//!    inverse gives each parameter's lower bound. [`tone_frequency_sigma_hz`]
//!    converts an audio SNR into the `sigma_hz` the caller supplies.
//! 2. **Position.** An event measures the perpendicular distance to its known
//!    track and the along-track position through the time of closest approach.
//!    With `u` the along-track unit vector and `n` the perpendicular one,
//!    `J_φ = Σ (n nᵀ / σ_d² + u uᵀ / (v² σ_t²))`, and the position bound is
//!    `1 / sqrt(λ_min(J_φ))`. One event leaves `J_φ` singular, which is the
//!    formal reason the localization stage refuses a single flyover (SDD
//!    section 6.5).
//!
//! The bound covers noise only. Absorption, refraction over long paths and
//! manoeuvres are model errors, and SDD section 7 says to compare the two
//! rather than fold them together.

use acoustics::{StraightPass, observed_frequency_hz};
use core_types::{Hertz, Metres};
use nalgebra::{SMatrix, SVector, SymmetricEigen, Vector2};

use crate::error::SimError;

/// Number of event parameters the frequency curve determines.
const PARAMETER_COUNT: usize = 4;

/// Relative step of a parameter's central difference, before the absolute
/// floor. A millionth of the parameter is far above `f64` rounding and far
/// below any curvature of the model over that step.
const RELATIVE_STEP: f64 = 1e-6;

/// Absolute floor of the time-offset step, seconds.
const TIME_STEP_S: f64 = 1e-6;

/// The acoustic geometry of one event, as an estimator parameterises it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventModel {
    /// Slant range at closest approach, metres; must be positive.
    pub closest_range_m: f64,
    /// Aircraft speed, metres per second; must be positive and below the
    /// speed of sound.
    pub speed_m_per_s: f64,
    /// Emitted fundamental frequency, hertz.
    pub fundamental_hz: f64,
    /// Speed of sound along the path, metres per second.
    pub sound_speed_m_per_s: f64,
}

impl EventModel {
    /// Check the geometry.
    ///
    /// Returns [`SimError::InvalidPass`] for non-positive or non-finite
    /// values, and [`SimError::Acoustics`] when the acoustics model rejects a
    /// supersonic source.
    fn validate(&self) -> Result<(), SimError> {
        let values = [
            self.closest_range_m,
            self.speed_m_per_s,
            self.fundamental_hz,
            self.sound_speed_m_per_s,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(SimError::InvalidPass { index: 0 });
        }
        StraightPass {
            closest_approach_m: self.closest_range_m,
            speed_m_per_s: self.speed_m_per_s,
        }
        .retarded_offset(0.0, self.sound_speed_m_per_s)?;
        Ok(())
    }

    /// Straight-pass geometry in the convention of the acoustics crate.
    fn pass(&self) -> StraightPass {
        StraightPass {
            closest_approach_m: self.closest_range_m,
            speed_m_per_s: self.speed_m_per_s,
        }
    }

    /// Observed fundamental frequency at a reception offset, hertz.
    ///
    /// `received_offset_s` is `u = t_r − t0e`, measured from the **emitted**
    /// closest approach, the convention of
    /// [`StraightPass::retarded_offset`](acoustics::StraightPass::retarded_offset).
    /// The heard closest approach therefore sits at `u = d / c`.
    ///
    /// Returns the errors of the acoustics model for impossible geometry.
    pub fn frequency_at(&self, received_offset_s: f64) -> Result<f64, SimError> {
        let pass = self.pass();
        let retarded = pass.retarded_offset(received_offset_s, self.sound_speed_m_per_s)?;
        let radial = pass.radial_speed(retarded)?;
        let heard =
            observed_frequency_hz(Hertz(self.fundamental_hz), radial, self.sound_speed_m_per_s)?;
        Ok(heard.0)
    }

    /// Gradient of the frequency curve with respect to
    /// `(f0, v, d, t0)` at a reception offset.
    ///
    /// Central differences: the frequency curve has no closed form for all
    /// four partials, and a step a millionth of the parameter keeps the
    /// truncation error far below the Fisher matrix's own conditioning.
    fn gradient(&self, received_offset_s: f64) -> Result<[f64; PARAMETER_COUNT], SimError> {
        let steps = [
            self.fundamental_hz * RELATIVE_STEP,
            self.speed_m_per_s * RELATIVE_STEP,
            self.closest_range_m * RELATIVE_STEP,
            TIME_STEP_S,
        ];
        let mut gradient = [0.0_f64; PARAMETER_COUNT];
        for (index, step) in steps.iter().enumerate() {
            let forward = self.with_perturbation(index, received_offset_s, *step)?;
            let backward = self.with_perturbation(index, received_offset_s, -*step)?;
            if let Some(slot) = gradient.get_mut(index) {
                *slot = (forward - backward) / (2.0 * step);
            }
        }
        Ok(gradient)
    }

    /// Frequency with parameter `index` moved by `delta`.
    ///
    /// The time parameter moves the curve rather than its value, because a
    /// timing error evaluates the same curve at a shifted offset.
    fn with_perturbation(
        &self,
        index: usize,
        received_offset_s: f64,
        delta: f64,
    ) -> Result<f64, SimError> {
        let mut moved = *self;
        let offset = match index {
            0 => {
                moved.fundamental_hz += delta;
                received_offset_s
            }
            1 => {
                moved.speed_m_per_s += delta;
                received_offset_s
            }
            2 => {
                moved.closest_range_m += delta;
                received_offset_s
            }
            _ => received_offset_s - delta,
        };
        moved.frequency_at(offset)
    }
}

/// The frequency noise of one event and the span it was measured over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObservationWindow {
    /// Analysis window length, seconds.
    pub duration_s: f64,
    /// Frequency estimates per second, hertz.
    pub sample_rate_hz: f64,
}

impl ObservationWindow {
    /// Number of frequency estimates in the window.
    ///
    /// Returns [`SimError::NotFinite`] for a non-finite or non-positive
    /// duration or rate, and for fewer than two samples.
    pub fn sample_count(&self) -> Result<usize, SimError> {
        if !self.duration_s.is_finite()
            || !self.sample_rate_hz.is_finite()
            || self.duration_s <= 0.0
            || self.sample_rate_hz <= 0.0
        {
            return Err(SimError::NotFinite);
        }
        let count = (self.duration_s * self.sample_rate_hz).round();
        if count < 2.0 || count > usize::MAX as f64 {
            return Err(SimError::NotFinite);
        }
        Ok(count as usize)
    }

    /// The sample offsets of a window centred on `centre_s`, in the
    /// reception-time convention of [`EventModel::frequency_at`].
    fn offsets(&self, centre_s: f64) -> Result<Vec<f64>, SimError> {
        let count = self.sample_count()?;
        let step_s = self.duration_s / count as f64;
        Ok((0..count)
            .map(|index| (index as f64 + 0.5) * step_s - 0.5 * self.duration_s + centre_s)
            .collect())
    }
}

/// The lower bound on each event parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventBound {
    /// Standard deviation of the emitted fundamental, hertz.
    pub sigma_fundamental_hz: f64,
    /// Standard deviation of the speed, metres per second.
    pub sigma_speed_m_per_s: f64,
    /// Standard deviation of the closest slant range, metres.
    pub sigma_closest_range_m: f64,
    /// Standard deviation of the time of closest approach, seconds.
    pub sigma_heard_time_s: f64,
}

/// Fisher information of one event's parameters, inverted into bounds.
///
/// `sigma_hz` is the standard deviation of the frequency measurement, which
/// [`tone_frequency_sigma_hz`] derives from an audio SNR. The frequency curve
/// is evaluated on the reception-time grid of `window`, which should be
/// centred on the heard closest approach.
///
/// Returns [`SimError::InvalidPass`] for impossible geometry,
/// [`SimError::InvalidNoise`] for a non-positive `sigma_hz` and
/// [`SimError::SingularFisher`] when the curve carries no information about
/// one of the parameters, for example a stationary aircraft.
pub fn event_bound(
    model: &EventModel,
    window: &ObservationWindow,
    sigma_hz: f64,
) -> Result<EventBound, SimError> {
    model.validate()?;
    if !sigma_hz.is_finite() || sigma_hz <= 0.0 {
        return Err(SimError::InvalidNoise(sigma_hz));
    }
    // The listener hears the pass at its lowest pitch `d / c` after the
    // aircraft is nearest, so that is where the analysis window sits.
    let delay_s = model.closest_range_m / model.sound_speed_m_per_s;
    let offsets = window.offsets(delay_s)?;
    let variance = sigma_hz * sigma_hz;

    let mut fisher = SMatrix::<f64, PARAMETER_COUNT, PARAMETER_COUNT>::zeros();
    for offset in offsets {
        let gradient = model.gradient(offset)?;
        let column = SVector::<f64, PARAMETER_COUNT>::from(gradient);
        fisher += column * column.transpose() / variance;
    }
    let covariance = fisher.try_inverse().ok_or(SimError::SingularFisher)?;

    Ok(EventBound {
        sigma_fundamental_hz: diagonal(&covariance, 0),
        sigma_speed_m_per_s: diagonal(&covariance, 1),
        sigma_closest_range_m: diagonal(&covariance, 2),
        sigma_heard_time_s: diagonal(&covariance, 3),
    })
}

/// One event's bound together with the direction of its track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventTrack {
    /// The event's parameter bound.
    pub bound: EventBound,
    /// Direction of flight, radians clockwise from north.
    pub azimuth_rad: f64,
    /// True speed along the track, metres per second.
    pub speed_m_per_s: f64,
}

/// The position bound of one set of events.
#[derive(Debug, Clone, PartialEq)]
pub struct CrlbReport {
    /// The per-event bounds that were combined.
    pub events: Vec<EventBound>,
    /// Square root of the largest position-covariance eigenvalue, metres.
    ///
    /// This is the semi-major axis of the 1σ ellipse. Infinite when the
    /// geometry is singular.
    pub position_sigma_m: Metres,
    /// Number of events that contributed.
    pub event_count: usize,
    /// Whether the geometry leaves some direction unbounded.
    pub is_singular: bool,
}
/// Combine event bounds into a position bound.
///
/// The unknowns are the observer's east and north offset and the recording
/// start, `φ = (east, north, s)`. Event `j` measures the perpendicular
/// distance to its track, `n_jᵀ p`, with variance `σ_d²`, and the along-track
/// position, `u_jᵀ p − v_j s`, with variance `(v_j σ_t)²`. The clock offset is
/// why one event is not enough: it absorbs the only time measurement, leaving
/// `J_φ` with rank two.
///
/// Returns [`SimError::InvalidPass`] for a non-finite azimuth or speed and for
/// a non-positive bound. A singular geometry is reported in the result with
/// `is_singular` set and an infinite sigma, not as an error.
pub fn position_bound(tracks: &[EventTrack]) -> Result<CrlbReport, SimError> {
    let mut fisher = SMatrix::<f64, 3, 3>::zeros();
    let mut events = Vec::with_capacity(tracks.len());

    for track in tracks {
        let bound = track.bound;
        let sigmas = [
            bound.sigma_fundamental_hz,
            bound.sigma_speed_m_per_s,
            bound.sigma_closest_range_m,
            bound.sigma_heard_time_s,
        ];
        if sigmas
            .iter()
            .any(|sigma| !sigma.is_finite() || *sigma <= 0.0)
            || !track.azimuth_rad.is_finite()
            || !track.speed_m_per_s.is_finite()
            || track.speed_m_per_s <= 0.0
        {
            return Err(SimError::InvalidPass { index: 0 });
        }

        // East-north components of the along-track and perpendicular
        // directions. A heading measured clockwise from north points along
        // (sin, cos).
        let along = Vector2::new(track.azimuth_rad.sin(), track.azimuth_rad.cos());
        let normal = Vector2::new(track.azimuth_rad.cos(), -track.azimuth_rad.sin());
        let distance_row = SVector::<f64, 3>::new(normal.x, normal.y, 0.0);
        let time_row = SVector::<f64, 3>::new(along.x, along.y, -track.speed_m_per_s);

        let range_variance = bound.sigma_closest_range_m * bound.sigma_closest_range_m;
        let along_variance_m = track.speed_m_per_s * bound.sigma_heard_time_s;
        fisher += distance_row * distance_row.transpose() / range_variance;
        fisher += time_row * time_row.transpose() / (along_variance_m * along_variance_m);
        events.push(bound);
    } // The three unknowns are in different units (metres, metres, seconds), so
    // the condition number of the raw Fisher matrix says nothing about the
    // geometry. Rescale to a correlation matrix first; a direction that is
    // nearly a linear combination of the others then shows up as a zero
    // eigenvalue here. Two parallel tracks differ only by sin π = 1.2e-16, so
    // an exact rank test would never fire.
    let scales = [
        component(&fisher, 0, 0),
        component(&fisher, 1, 1),
        component(&fisher, 2, 2),
    ];
    if scales
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Ok(singular_report(events));
    }
    let mut inverse_scales = SVector::<f64, 3>::zeros();
    for (index, value) in scales.iter().enumerate() {
        if let Some(slot) = inverse_scales.get_mut(index) {
            *slot = 1.0 / value.sqrt();
        }
    }
    let scaling = SMatrix::<f64, 3, 3>::from_diagonal(&inverse_scales);
    let correlation = scaling * fisher * scaling;
    let smallest = SymmetricEigen::new(correlation)
        .eigenvalues
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    if !smallest.is_finite() || smallest <= 1e-10 {
        return Ok(singular_report(events));
    }
    let Some(covariance) = fisher.try_inverse() else {
        return Ok(singular_report(events));
    };
    let a = component(&covariance, 0, 0);
    let b = component(&covariance, 0, 1);
    let c = component(&covariance, 1, 1);
    let semi_major = largest_eigenvalue(a, b, c);
    if !semi_major.is_finite() || semi_major < 0.0 {
        return Ok(singular_report(events));
    }

    Ok(CrlbReport {
        events,
        position_sigma_m: Metres(semi_major.sqrt()),
        event_count: tracks.len(),
        is_singular: false,
    })
}

/// A report with every event but no bounded direction.
fn singular_report(events: Vec<EventBound>) -> CrlbReport {
    CrlbReport {
        event_count: events.len(),
        events,
        position_sigma_m: Metres(f64::INFINITY),
        is_singular: true,
    }
}

/// Largest eigenvalue of the symmetric 2x2 matrix `[[a, b], [b, c]]`.
fn largest_eigenvalue(a: f64, b: f64, c: f64) -> f64 {
    let half_trace = f64::midpoint(a, c);
    let spread = (0.25 * (a - c) * (a - c) + b * b).sqrt();
    half_trace + spread
}

/// Frequency standard deviation of a tone in white noise.
///
/// For `N` samples of amplitude `A` in noise of RMS `sigma`, the Fisher
/// information of the angular frequency is `A² N (N² − 1) / (24 σ²)`, so
/// `sigma_f = (1 / (π A)) sqrt(6 σ² / (N (N² − 1)))`. The bound assumes a
/// single tone observed for the whole window, which is why an event is
/// resolved into harmonics before it is bounded.
///
/// Returns [`SimError::NotFinite`] for a non-finite or non-positive amplitude
/// or noise level, and for fewer than two samples.
pub fn tone_frequency_sigma_hz(
    peak_amplitude: f64,
    noise_rms: f64,
    sample_count: usize,
) -> Result<f64, SimError> {
    if !peak_amplitude.is_finite()
        || !noise_rms.is_finite()
        || peak_amplitude <= 0.0
        || noise_rms <= 0.0
        || sample_count < 2
    {
        return Err(SimError::NotFinite);
    }
    let count = sample_count as f64;
    let variance = 6.0 * noise_rms * noise_rms / (count * (count * count - 1.0));
    Ok(variance.sqrt() / (core::f64::consts::PI * peak_amplitude))
}

/// One entry of a matrix, as `f64`.
fn component(matrix: &SMatrix<f64, 3, 3>, row: usize, column: usize) -> f64 {
    matrix.get((row, column)).copied().unwrap_or(f64::NAN)
}

/// Square root of one diagonal entry, infinite when it is not usable.
fn diagonal(matrix: &SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>, index: usize) -> f64 {
    matrix
        .get((index, index))
        .copied()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map_or(f64::INFINITY, f64::sqrt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use acoustics::Atmosphere;

    /// Relative tolerance for ratios of bounds. Both sides come from the same
    /// Fisher matrix, so only the finite-difference step separates them.
    const REL_TOL: f64 = 1e-9;

    fn model() -> EventModel {
        EventModel {
            closest_range_m: 800.0_f64.hypot(600.0 - 1.7),
            speed_m_per_s: 90.0,
            fundamental_hz: 100.0,
            sound_speed_m_per_s: Atmosphere::standard().speed_of_sound_m_per_s(),
        }
    }

    fn window(duration_s: f64) -> ObservationWindow {
        ObservationWindow {
            duration_s,
            sample_rate_hz: 1_000.0,
        }
    }

    /// A ten-second window at 0.1 Hz of frequency noise. A one-second window
    /// centred on the pass is too symmetric for the time and distance to be
    /// identifiable, and the bound correctly reports that as a huge variance.
    fn bound() -> EventBound {
        event_bound(&model(), &window(10.0), 0.1).expect("valid bound")
    }

    #[test]
    fn frequency_curve_passes_through_the_emitted_pitch() {
        // At the heard closest approach, u = d / c, the radial speed is zero,
        // so the emitted fundamental is heard unchanged (SDD section 4).
        let model = model();
        let delay = model.closest_range_m / model.sound_speed_m_per_s;
        let heard = model.frequency_at(delay).expect("valid geometry");
        assert!(
            (heard - model.fundamental_hz).abs() < 1e-6,
            "expected {} Hz, got {heard}",
            model.fundamental_hz
        );
    }

    #[test]
    fn event_bound_is_finite_and_physical() {
        let bound = bound();
        assert!(bound.sigma_fundamental_hz > 0.0 && bound.sigma_fundamental_hz.is_finite());
        assert!(bound.sigma_speed_m_per_s > 0.0 && bound.sigma_speed_m_per_s.is_finite());
        assert!(bound.sigma_closest_range_m > 0.0 && bound.sigma_closest_range_m.is_finite());
        assert!(bound.sigma_heard_time_s > 0.0 && bound.sigma_heard_time_s.is_finite());
        // The fundamental is the best-measured parameter; the distance is the
        // worst, because the curve is nearly symmetric about the pass.
        assert!(bound.sigma_fundamental_hz < 0.1, "got {bound:?}");
        assert!(bound.sigma_speed_m_per_s < 1.0, "got {bound:?}");
        assert!(bound.sigma_closest_range_m < 20.0, "got {bound:?}");
        assert!(bound.sigma_heard_time_s < 0.1, "got {bound:?}");
    }

    #[test]
    fn bounds_scale_linearly_with_frequency_noise() {
        let single = event_bound(&model(), &window(10.0), 0.1).expect("valid");
        let double = event_bound(&model(), &window(10.0), 0.2).expect("valid");
        let ratios = [
            (double.sigma_fundamental_hz, single.sigma_fundamental_hz),
            (double.sigma_speed_m_per_s, single.sigma_speed_m_per_s),
            (double.sigma_closest_range_m, single.sigma_closest_range_m),
            (double.sigma_heard_time_s, single.sigma_heard_time_s),
        ];
        for (doubled, base) in ratios {
            assert!(
                (doubled / base - 2.0).abs() < REL_TOL * 2.0,
                "doubling the noise must double the bound: {doubled} vs {base}"
            );
        }
    }

    #[test]
    fn a_longer_window_tightens_the_distance_bound() {
        let short = event_bound(&model(), &window(5.0), 0.1).expect("valid");
        let long = event_bound(&model(), &window(20.0), 0.1).expect("valid");
        assert!(long.sigma_closest_range_m < short.sigma_closest_range_m);
        assert!(long.sigma_speed_m_per_s < short.sigma_speed_m_per_s);
    }

    #[test]
    fn a_stationary_source_leaves_the_speed_unbounded() {
        let mut stationary = model();
        stationary.speed_m_per_s = 0.0;
        // A zero speed fails validation before the Fisher matrix is even
        // reached, which is the honest answer: the model itself degenerates.
        assert!(matches!(
            event_bound(&stationary, &window(1.0), 0.5),
            Err(SimError::InvalidPass { index: 0 })
        ));
    }

    #[test]
    fn impossible_inputs_are_rejected() {
        let mut negative_range = model();
        negative_range.closest_range_m = -1.0;
        assert!(matches!(
            event_bound(&negative_range, &window(1.0), 0.5),
            Err(SimError::InvalidPass { index: 0 })
        ));
        assert!(matches!(
            event_bound(&model(), &window(1.0), 0.0),
            Err(SimError::InvalidNoise(_))
        ));
        let empty = ObservationWindow {
            duration_s: 0.001,
            sample_rate_hz: 10.0,
        };
        assert!(matches!(
            event_bound(&model(), &empty, 0.5),
            Err(SimError::NotFinite)
        ));
    }

    #[test]
    fn one_event_leaves_the_position_unbounded() {
        let track = EventTrack {
            bound: bound(),
            azimuth_rad: 0.0,
            speed_m_per_s: 90.0,
        };
        let report = position_bound(&[track]).expect("valid");
        assert!(report.is_singular);
        assert!(report.position_sigma_m.0.is_infinite());
        assert_eq!(report.event_count, 1);
    }

    #[test]
    fn duplicate_tracks_stay_singular() {
        // Hearing the same pass twice adds no information: the second row is
        // the first one again, leaving the along-track position and the clock
        // offset indistinguishable.
        let base = EventTrack {
            bound: bound(),
            azimuth_rad: 0.0,
            speed_m_per_s: 90.0,
        };
        let report = position_bound(&[base, base]).expect("valid");
        assert!(report.is_singular, "a repeated track fixes nothing new");
    }

    #[test]
    fn antiparallel_tracks_resolve_the_clock_offset() {
        // A pass in the opposite direction maps along-track position to time
        // with the other sign, so the clock offset and the position no longer
        // trade off against each other.
        let northbound = EventTrack {
            bound: bound(),
            azimuth_rad: 0.0,
            speed_m_per_s: 90.0,
        };
        let southbound = EventTrack {
            azimuth_rad: std::f64::consts::PI,
            ..northbound
        };
        let report = position_bound(&[northbound, southbound]).expect("valid");
        assert!(!report.is_singular);
        assert!(report.position_sigma_m.0.is_finite());
    }

    #[test]
    fn orthogonal_tracks_bound_the_position() {
        // Two crossed tracks each pin one direction, so the ellipse is finite
        // and smaller than either distance bound alone.
        let event = EventBound {
            sigma_fundamental_hz: 0.1,
            sigma_speed_m_per_s: 1.0,
            sigma_closest_range_m: 10.0,
            sigma_heard_time_s: 0.02,
        };
        let east = EventTrack {
            bound: event,
            azimuth_rad: std::f64::consts::FRAC_PI_2,
            speed_m_per_s: 90.0,
        };
        let north = EventTrack {
            bound: event,
            azimuth_rad: 0.0,
            speed_m_per_s: 90.0,
        };
        let report = position_bound(&[east, north]).expect("valid");
        assert!(!report.is_singular);
        assert!(
            report.position_sigma_m.0 > 1.0 && report.position_sigma_m.0 < 10.0,
            "expected a bounded, tighter ellipse, got {}",
            report.position_sigma_m.0
        );
        assert_eq!(report.events.len(), 2);
    }

    #[test]
    fn a_better_event_halves_the_position_bound() {
        let base = EventBound {
            sigma_fundamental_hz: 0.1,
            sigma_speed_m_per_s: 1.0,
            sigma_closest_range_m: 20.0,
            sigma_heard_time_s: 0.01,
        };
        let tight = EventBound {
            sigma_closest_range_m: 10.0,
            ..base
        };
        let tracks = |event| {
            [
                EventTrack {
                    bound: event,
                    azimuth_rad: std::f64::consts::FRAC_PI_2,
                    speed_m_per_s: 90.0,
                },
                EventTrack {
                    bound: event,
                    azimuth_rad: 0.0,
                    speed_m_per_s: 90.0,
                },
            ]
        };
        let coarse = position_bound(&tracks(base)).expect("valid");
        let fine = position_bound(&tracks(tight)).expect("valid");
        let ratio = coarse.position_sigma_m.0 / fine.position_sigma_m.0;
        // The distance bound dominates the mixture, so halving it nearly
        // halves the ellipse; the timing term keeps the ratio below two.
        assert!(
            (1.5..2.0).contains(&ratio),
            "expected the ellipse to shrink by about two, got {ratio}"
        );
    }

    #[test]
    fn position_bound_rejects_impossible_tracks() {
        let mut track = EventTrack {
            bound: bound(),
            azimuth_rad: 0.0,
            speed_m_per_s: 90.0,
        };
        track.speed_m_per_s = 0.0;
        assert!(matches!(
            position_bound(&[track]),
            Err(SimError::InvalidPass { index: 0 })
        ));
    }

    #[test]
    fn tone_frequency_bound_follows_the_sample_count() {
        let short = tone_frequency_sigma_hz(0.5, 0.05, 1_000).expect("valid");
        let long = tone_frequency_sigma_hz(0.5, 0.05, 2_000).expect("valid");
        // The bound goes as N^(-3/2) for large N, so eight times the samples
        // is 2^(3/2) = 2.83 times the precision.
        let ratio = short / long;
        assert!(
            (ratio - 2.0_f64.powf(1.5)).abs() < 0.01 * ratio,
            "expected 2.83, got {ratio}"
        );
    }

    #[test]
    fn tone_frequency_bound_is_linear_in_amplitude_and_noise() {
        let base = tone_frequency_sigma_hz(0.5, 0.05, 8_000).expect("valid");
        let louder = tone_frequency_sigma_hz(1.0, 0.05, 8_000).expect("valid");
        let noisier = tone_frequency_sigma_hz(0.5, 0.10, 8_000).expect("valid");
        assert!((louder / base - 0.5).abs() < REL_TOL);
        assert!((noisier / base - 2.0).abs() < REL_TOL);
        assert!(matches!(
            tone_frequency_sigma_hz(0.0, 0.05, 8_000),
            Err(SimError::NotFinite)
        ));
        assert!(matches!(
            tone_frequency_sigma_hz(0.5, 0.05, 1),
            Err(SimError::NotFinite)
        ));
    }
}
