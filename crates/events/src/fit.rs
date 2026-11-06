//! Per-event observations and fitted parameters (FR-DM-02 to FR-DM-05).
//!
//! The track-then-fit estimator of `docs/SDD.md` section 5.5 fits the four
//! event parameters `(f0, v, d, t0r)` jointly to every visible harmonic. The
//! model is the emission-time Doppler curve of section 4, supplied by
//! [`acoustics::StraightPass`]; a delayed copy of the received curve is not an
//! acceptable substitute (FR-DM-03).
//!
//! Each track point carries its own frequency variance, so the least-squares
//! problem is weighted. A Huber loss keeps one bad ridge frame from dominating
//! the fit. The covariance is `(JᵀWJ)⁻¹` scaled by `max(1, χ²/dof)`, and where
//! speed and closest distance are strongly correlated the Gaussian ellipse is
//! replaced by a profile-likelihood interval (FR-DM-02, FR-DM-04).

use acoustics::StraightPass;
use core_types::{AudioSeconds, EventId, Hertz, Metres};
use dsp::{Ridge, ZMap};
use nalgebra::{SMatrix, SVector};

use crate::detect::EventHypothesis;
use crate::error::EventError;

/// Number of fitted event parameters: `(f0, v, d, t0r)`.
const PARAMETER_COUNT: usize = 4;
/// Position of the fundamental frequency in the parameter vector.
const PARAM_FUNDAMENTAL: usize = 0;
/// Position of the aircraft speed in the parameter vector.
const PARAM_SPEED: usize = 1;
/// Position of the closest-approach distance in the parameter vector.
const PARAM_CLOSEST: usize = 2;

/// Relative step of a parameter's central difference.
///
/// A millionth of the parameter is far above `f64` rounding and far below the
/// curvature of the model over that step. It matches the step the Cramér–Rao
/// bound in `sim` uses (FR-SIM-03).
const RELATIVE_STEP: f64 = 1e-6;
/// Absolute step of the time parameter, seconds.
const TIME_STEP_S: f64 = 1e-6;
/// Largest damping the Levenberg–Marquardt loop will try before giving up.
const MAX_LAMBDA: f64 = 1.0e12;
/// Smallest damping kept after an accepted step.
const MIN_LAMBDA: f64 = 1.0e-12;
/// Attempts at one damped step before the loop treats it as stuck.
const MAX_INNER_STEPS: usize = 24;
/// Backtracking halvings of a step before the damping is raised instead.
const MAX_BACKTRACK_STEPS: usize = 10;
/// Outward probes of one profile-likelihood side.
const MAX_PROFILE_STEPS: usize = 24;
/// Bisection refinements once a profile boundary has been bracketed.
const PROFILE_BISECTIONS: usize = 24;

/// One observed frequency sample of a harmonic ridge.
///
/// `heard_at` is the centre time of the STFT frame on the audio clock, seconds.
/// `frequency_variance_hz2` is the variance of that frequency estimate, in
/// hertz squared, and must be positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    /// Frame centre on the audio clock, seconds.
    pub heard_at: AudioSeconds,
    /// Observed frequency of the harmonic, hertz.
    pub frequency_hz: Hertz,
    /// Variance of the observed frequency, hertz squared.
    pub frequency_variance_hz2: f64,
}

impl TrackPoint {
    /// Build a point and validate its units.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite time or frequency,
    /// and [`EventError::InvalidVariance`] for a variance that is not positive
    /// and finite.
    pub fn new(
        heard_at: AudioSeconds,
        frequency_hz: Hertz,
        frequency_variance_hz2: f64,
    ) -> Result<Self, EventError> {
        let point = Self {
            heard_at,
            frequency_hz,
            frequency_variance_hz2,
        };
        point.validate()?;
        Ok(point)
    }

    /// Check the point's units.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.heard_at.0.is_finite() || !self.frequency_hz.0.is_finite() {
            return Err(EventError::NotFinite);
        }
        if self.frequency_hz.0 <= 0.0 {
            return Err(EventError::InvalidVariance(self.frequency_hz.0));
        }
        if !self.frequency_variance_hz2.is_finite() || self.frequency_variance_hz2 <= 0.0 {
            return Err(EventError::InvalidVariance(self.frequency_variance_hz2));
        }
        Ok(())
    }
}

/// One harmonic's tracked ridge, ready for the joint fit.
///
/// A harmonic index of one is the fundamental. Every point must carry its own
/// variance so the fit can weight the frames (SDD section 5.5).
#[derive(Debug, Clone, PartialEq)]
pub struct HarmonicTrack {
    /// Harmonic index, one for the fundamental.
    pub harmonic: u32,
    /// Ridge samples in increasing or arbitrary time order.
    pub points: Vec<TrackPoint>,
}

impl HarmonicTrack {
    /// Build a track and check its harmonic and points.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidHarmonic`] for harmonic zero,
    /// [`EventError::ZeroLength`] for an empty point list, and the errors of
    /// [`TrackPoint::validate`].
    pub fn new(harmonic: u32, points: Vec<TrackPoint>) -> Result<Self, EventError> {
        if harmonic == 0 {
            return Err(EventError::InvalidHarmonic(harmonic));
        }
        if points.is_empty() {
            return Err(EventError::ZeroLength);
        }
        for point in &points {
            point.validate()?;
        }
        Ok(Self { harmonic, points })
    }

    /// Convert a tracked ridge on a whitened map into fit samples.
    ///
    /// Frame times come from the map, so the audio clock matches the
    /// spectrogram. A smoothed ridge carries per-frame variances in bins
    /// squared; the other ladder levels do not, and use
    /// `fallback_variance_bins2` instead. Both are converted to hertz squared
    /// with the map's bin width.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::LengthMismatch`] when the ridge does not have one
    /// point per map frame or a variance per point, and
    /// [`EventError::InvalidVariance`] for a non-positive fallback.
    pub fn from_ridge(
        harmonic: u32,
        ridge: &Ridge,
        map: &ZMap,
        fallback_variance_bins2: f64,
    ) -> Result<Self, EventError> {
        let frames = map.frame_count();
        if ridge.frame_count() != frames {
            return Err(EventError::LengthMismatch {
                expected: frames,
                actual: ridge.frame_count(),
            });
        }
        if !fallback_variance_bins2.is_finite() || fallback_variance_bins2 <= 0.0 {
            return Err(EventError::InvalidVariance(fallback_variance_bins2));
        }
        let variances = ridge.variances();
        if let Some(reported) = variances
            && reported.len() != frames
        {
            return Err(EventError::LengthMismatch {
                expected: frames,
                actual: reported.len(),
            });
        }
        let width_hz = map.config().bin_width_hz();
        let variance_scale = width_hz * width_hz;
        let times = map.frame_times_s();
        let mut points = Vec::with_capacity(frames);
        for (frame, point) in ridge.points().iter().enumerate() {
            let Some(&heard_at) = times.get(frame) else {
                return Err(EventError::LengthMismatch {
                    expected: frames,
                    actual: times.len(),
                });
            };
            let variance_bins2 = match variances {
                Some(reported) => {
                    let Some(&variance) = reported.get(frame) else {
                        return Err(EventError::LengthMismatch {
                            expected: frames,
                            actual: reported.len(),
                        });
                    };
                    variance
                }
                None => fallback_variance_bins2,
            };
            points.push(TrackPoint::new(
                AudioSeconds(heard_at),
                Hertz(point.frequency_hz),
                variance_bins2 * variance_scale,
            )?);
        }
        Self::new(harmonic, points)
    }

    /// Number of points in the track.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Return `true` when the track holds no points.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// Settings for the track-then-fit estimator.
///
/// The defaults are `PROVISIONAL(G1)`: the simulator suite fixes the robust
/// cut, the correlation threshold and the profile confidence level once the
/// estimator can be scored against truth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitConfig {
    /// Speed of sound used by the emission-time model, metres per second.
    pub sound_speed_m_per_s: f64,
    /// Huber cut in standard deviations; residuals beyond it are down-weighted.
    pub huber_k: f64,
    /// Maximum Levenberg–Marquardt iterations per start.
    pub max_iterations: usize,
    /// Relative parameter change below which the optimizer stops.
    pub step_tolerance: f64,
    /// Initial Levenberg–Marquardt damping.
    pub lambda_initial: f64,
    /// Factor by which damping changes after a rejected or accepted step.
    pub lambda_factor: f64,
    /// Points every harmonic track must contribute.
    pub min_points_per_harmonic: usize,
    /// Absolute speed/closest correlation above which profiles replace the ellipse.
    pub profile_correlation_threshold: f64,
    /// Change in weighted `χ²` that defines a one-parameter confidence bound.
    ///
    /// The default is the 95 % point of a one-degree-of-freedom chi-square
    /// distribution, `3.841458820694124`, and is not a provisional tuning value.
    pub profile_delta_chi_squared: f64,
    /// Speed multipliers used to build multi-start seeds around the detector.
    pub speed_start_scales: [f64; 3],
    /// Closest-distance multipliers used to build multi-start seeds.
    pub closest_start_scales: [f64; 3],
}

impl Default for FitConfig {
    fn default() -> Self {
        Self {
            sound_speed_m_per_s: 343.0,
            huber_k: 3.0,
            max_iterations: 100,
            step_tolerance: 1.0e-9,
            lambda_initial: 1.0e-3,
            lambda_factor: 10.0,
            min_points_per_harmonic: 3,
            profile_correlation_threshold: 0.8,
            profile_delta_chi_squared: 3.841_458_820_694_124,
            speed_start_scales: [0.8, 1.0, 1.2],
            closest_start_scales: [0.7, 1.0, 1.4],
        }
    }
}

impl FitConfig {
    /// Check the settings before a fit.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite sound speed, and
    /// [`EventError::InvalidRate`] for a non-positive cut, damping or scale, a
    /// damping factor at or below one, a threshold outside `[0, 1]`, or a
    /// non-positive profile step. Returns [`EventError::ZeroLength`] for a zero
    /// iteration count or minimum point count.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.sound_speed_m_per_s.is_finite() || self.sound_speed_m_per_s <= 0.0 {
            return Err(EventError::NotFinite);
        }
        if !self.huber_k.is_finite() || self.huber_k <= 0.0 {
            return Err(EventError::InvalidRate(self.huber_k));
        }
        if self.max_iterations == 0 || self.min_points_per_harmonic == 0 {
            return Err(EventError::ZeroLength);
        }
        if !self.step_tolerance.is_finite() || self.step_tolerance <= 0.0 {
            return Err(EventError::InvalidRate(self.step_tolerance));
        }
        if !self.lambda_initial.is_finite() || self.lambda_initial <= 0.0 {
            return Err(EventError::InvalidRate(self.lambda_initial));
        }
        if !self.lambda_factor.is_finite() || self.lambda_factor <= 1.0 {
            return Err(EventError::InvalidRate(self.lambda_factor));
        }
        if !self.profile_correlation_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.profile_correlation_threshold)
        {
            return Err(EventError::InvalidRate(self.profile_correlation_threshold));
        }
        if !self.profile_delta_chi_squared.is_finite() || self.profile_delta_chi_squared <= 0.0 {
            return Err(EventError::InvalidRate(self.profile_delta_chi_squared));
        }
        for scale in self
            .speed_start_scales
            .iter()
            .chain(self.closest_start_scales.iter())
        {
            if !scale.is_finite() || *scale <= 0.0 {
                return Err(EventError::InvalidRate(*scale));
            }
        }
        Ok(())
    }
}

/// Which estimator produced a fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMethod {
    /// Joint least squares on per-harmonic ridge samples (EVT-02).
    TrackThenFit,
    /// Direct generalized likelihood ratio on the whitened map (EVT-03).
    Direct,
}

/// A one-parameter profile-likelihood confidence interval.
///
/// Values are in the parameter's own unit: hertz for the fundamental, metres
/// per second for speed, metres for distance and seconds for time. A side is
/// `None` when the profile did not reach the confidence threshold before a
/// physical bound or the probe budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileInterval {
    /// Lower endpoint, or `None` when the profile is open downwards.
    pub lower: Option<f64>,
    /// Upper endpoint, or `None` when the profile is open upwards.
    pub upper: Option<f64>,
    /// Change in weighted `χ²` that defined the endpoints.
    pub delta_chi_squared: f64,
}

/// A fitted event together with the evidence used to judge it.
///
/// The `EventFit` is the part later stages consume; the remaining fields are
/// diagnostics for the quality score (FR-DM-06) and the research report.
#[derive(Debug, Clone, PartialEq)]
pub struct FitReport {
    /// Fitted parameters and covariance.
    pub fit: EventFit,
    /// Estimator that produced the fit.
    pub method: FitMethod,
    /// Weighted residual sum of squares at the optimum.
    pub chi_squared: f64,
    /// Observations minus fitted parameters; always at least one.
    pub degrees_of_freedom: usize,
    /// `max(1, χ²/dof)` applied to the covariance (SDD section 5.5).
    pub covariance_scale: f64,
    /// Huber objective at the optimum, in weighted residual units.
    pub robust_cost: f64,
    /// Correlation of the speed and closest-distance estimates.
    pub speed_closest_correlation: f64,
    /// Profile interval for speed, present only when the correlation is high.
    pub speed_interval: Option<ProfileInterval>,
    /// Profile interval for closest distance, present with the speed interval.
    pub closest_interval: Option<ProfileInterval>,
    /// Levenberg–Marquardt iterations used by the winning start.
    pub iterations: usize,
    /// Number of observations in the joint fit.
    pub observations: usize,
    /// Harmonics that contributed, in ascending order.
    pub harmonics: Vec<u32>,
}

/// One frequency measurement after the tracks were flattened.
#[derive(Debug, Clone, Copy)]
struct Observation {
    heard_at_s: f64,
    harmonic: u32,
    frequency_hz: f64,
    variance_hz2: f64,
}

/// The four fitted parameters, in a named form that avoids unchecked indexing.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Parameters {
    fundamental_hz: f64,
    speed_m_per_s: f64,
    closest_m: f64,
    heard_at_s: f64,
}

impl Parameters {
    /// Seed the optimizer from a detector hypothesis.
    fn from_hypothesis(seed: &EventHypothesis) -> Self {
        Self {
            fundamental_hz: seed.fundamental_hz.0,
            speed_m_per_s: seed.speed_m_per_s,
            closest_m: seed.closest_m,
            heard_at_s: seed.heard_at.0,
        }
    }

    /// One parameter by vector position.
    fn get(self, index: usize) -> f64 {
        match index {
            PARAM_FUNDAMENTAL => self.fundamental_hz,
            PARAM_SPEED => self.speed_m_per_s,
            PARAM_CLOSEST => self.closest_m,
            _ => self.heard_at_s,
        }
    }

    /// Write one parameter by vector position.
    fn set(&mut self, index: usize, value: f64) {
        match index {
            PARAM_FUNDAMENTAL => self.fundamental_hz = value,
            PARAM_SPEED => self.speed_m_per_s = value,
            PARAM_CLOSEST => self.closest_m = value,
            _ => self.heard_at_s = value,
        }
    }

    /// Move one parameter by `delta`.
    fn add(&mut self, index: usize, delta: f64) {
        self.set(index, self.get(index) + delta);
    }

    /// The same parameters with one entry moved.
    fn with_added(mut self, index: usize, delta: f64) -> Self {
        self.add(index, delta);
        self
    }

    /// Whether the geometry admits the subsonic emission-time model.
    fn is_physical(self, sound_speed_m_per_s: f64) -> bool {
        self.fundamental_hz.is_finite()
            && self.speed_m_per_s.is_finite()
            && self.closest_m.is_finite()
            && self.heard_at_s.is_finite()
            && self.fundamental_hz > 0.0
            && self.speed_m_per_s > 0.0
            && self.speed_m_per_s < sound_speed_m_per_s
            && self.closest_m > 0.0
    }
}

/// Weighted residual statistics at one parameter vector.
#[derive(Debug, Clone)]
struct Evaluation {
    cost: f64,
    chi_squared: f64,
    residuals: Vec<f64>,
    weights: Vec<f64>,
}

/// The best point found by one optimization run.
#[derive(Debug, Clone, Copy)]
struct Optimum {
    parameters: Parameters,
    cost: f64,
    iterations: usize,
}

/// Build multi-start seeds around one detector hypothesis.
///
/// The detector slides speed and closest distance on a coarse grid, so the
/// optimizer is given every combination of the configured multipliers around
/// its best hypothesis. The fundamental and heard time are kept from the seed.
pub fn multi_start_seeds(seed: &EventHypothesis, config: &FitConfig) -> Vec<EventHypothesis> {
    let mut seeds = Vec::with_capacity(
        config
            .speed_start_scales
            .len()
            .saturating_mul(config.closest_start_scales.len()),
    );
    for speed_scale in config.speed_start_scales {
        for closest_scale in config.closest_start_scales {
            let candidate = EventHypothesis {
                heard_at: seed.heard_at,
                fundamental_hz: seed.fundamental_hz,
                speed_m_per_s: seed.speed_m_per_s * speed_scale,
                closest_m: seed.closest_m * closest_scale,
            };
            if Parameters::from_hypothesis(&candidate).is_physical(config.sound_speed_m_per_s)
                && !seeds.contains(&candidate)
            {
                seeds.push(candidate);
            }
        }
    }
    seeds
}

/// Fit one event with the track-then-fit estimator.
///
/// The detector hypothesis seeds a multi-start search over the speed and
/// closest-distance multipliers in `config`. The best robust objective wins.
///
/// # Errors
///
/// Returns the errors of [`fit_event_from_seeds`], and
/// [`EventError::NoHypothesis`] when no seed is physical or none converges.
pub fn fit_event(
    tracks: &[HarmonicTrack],
    event_id: EventId,
    seed: &EventHypothesis,
    config: &FitConfig,
) -> Result<FitReport, EventError> {
    let seeds = multi_start_seeds(seed, config);
    fit_event_from_seeds(tracks, event_id, &seeds, config)
}

/// Fit one event from an explicit list of starting hypotheses.
///
/// Each seed is optimized independently and the lowest robust objective is
/// kept. A seed is skipped when it is not physical or the optimizer cannot
/// evaluate it; the fit fails only when every seed is skipped.
///
/// # Errors
///
/// Returns [`EventError::ZeroLength`] for no tracks, [`EventError::NotFinite`]
/// or [`EventError::InvalidVariance`] for bad samples,
/// [`EventError::DuplicateHarmonic`] for a repeated harmonic,
/// [`EventError::InsufficientTrackPoints`] for a short track,
/// [`EventError::InsufficientObservations`] when the fit has no residual
/// degrees of freedom, [`EventError::SingularFit`] when the normal equations
/// cannot be inverted, and [`EventError::NoHypothesis`] when no seed works.
pub fn fit_event_from_seeds(
    tracks: &[HarmonicTrack],
    event_id: EventId,
    seeds: &[EventHypothesis],
    config: &FitConfig,
) -> Result<FitReport, EventError> {
    config.validate()?;
    let observations = collect_observations(tracks, config)?;
    let count = observations.len();
    if count <= PARAMETER_COUNT {
        return Err(EventError::InsufficientObservations {
            needed: PARAMETER_COUNT,
            actual: count,
        });
    }

    let mut best: Option<Optimum> = None;
    for seed in seeds {
        let initial = Parameters::from_hypothesis(seed);
        if !initial.is_physical(config.sound_speed_m_per_s) {
            continue;
        }
        let Ok(optimum) = optimize(initial, &observations, config, None, None) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|current| optimum.cost < current.cost)
        {
            best = Some(optimum);
        }
    }
    let best = best.ok_or(EventError::NoHypothesis)?;

    let evaluation = evaluate(&best.parameters, &observations, config, None)?;
    let (information, _) =
        information_matrix(&best.parameters, &observations, config, &evaluation.weights)?;
    let inverse = information.try_inverse().ok_or(EventError::SingularFit)?;
    let degrees_of_freedom = count.saturating_sub(PARAMETER_COUNT);
    if degrees_of_freedom == 0 {
        return Err(EventError::InsufficientObservations {
            needed: PARAMETER_COUNT,
            actual: count,
        });
    }
    let covariance_scale = (evaluation.chi_squared / degrees_of_freedom as f64).max(1.0);
    if !covariance_scale.is_finite() {
        return Err(EventError::NotFinite);
    }
    let covariance = inverse * covariance_scale;
    let speed_closest_correlation = correlation(&covariance)?;

    let (speed_interval, closest_interval) =
        if speed_closest_correlation.abs() >= config.profile_correlation_threshold {
            let speed = profile_interval(
                PARAM_SPEED,
                &best.parameters,
                &evaluation.weights,
                &observations,
                config,
            );
            let closest = profile_interval(
                PARAM_CLOSEST,
                &best.parameters,
                &evaluation.weights,
                &observations,
                config,
            );
            (speed, closest)
        } else {
            (None, None)
        };

    let mut harmonics: Vec<u32> = tracks.iter().map(|track| track.harmonic).collect();
    harmonics.sort_unstable();

    Ok(FitReport {
        fit: EventFit {
            event_id,
            fundamental_hz: Hertz(best.parameters.fundamental_hz),
            speed_m_per_s: best.parameters.speed_m_per_s,
            closest_m: Metres(best.parameters.closest_m),
            heard_at: AudioSeconds(best.parameters.heard_at_s),
            covariance: covariance_array(&covariance),
        },
        method: FitMethod::TrackThenFit,
        chi_squared: evaluation.chi_squared,
        degrees_of_freedom,
        covariance_scale,
        robust_cost: best.cost,
        speed_closest_correlation,
        speed_interval,
        closest_interval,
        iterations: best.iterations,
        observations: count,
        harmonics,
    })
}

/// A detected event with its harmonic assignment.
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
/// heard_time_s)` in that order. The covariance is the inverse Hessian from the
/// fit, scaled as described in `docs/SDD.md` section 5.5 (FR-DM-02).
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
    /// Symmetric 4x4 parameter covariance in the order above.
    pub covariance: [[f64; 4]; 4],
}

/// Flatten and validate the tracks.
fn collect_observations(
    tracks: &[HarmonicTrack],
    config: &FitConfig,
) -> Result<Vec<Observation>, EventError> {
    if tracks.is_empty() {
        return Err(EventError::ZeroLength);
    }
    let mut seen: Vec<u32> = Vec::with_capacity(tracks.len());
    let mut observations = Vec::new();
    for track in tracks {
        if track.harmonic == 0 {
            return Err(EventError::InvalidHarmonic(track.harmonic));
        }
        if seen.contains(&track.harmonic) {
            return Err(EventError::DuplicateHarmonic(track.harmonic));
        }
        seen.push(track.harmonic);
        if track.points.len() < config.min_points_per_harmonic {
            return Err(EventError::InsufficientTrackPoints {
                harmonic: track.harmonic,
                needed: config.min_points_per_harmonic,
                actual: track.points.len(),
            });
        }
        for point in &track.points {
            point.validate()?;
            observations.push(Observation {
                heard_at_s: point.heard_at.0,
                harmonic: track.harmonic,
                frequency_hz: point.frequency_hz.0,
                variance_hz2: point.frequency_variance_hz2,
            });
        }
    }
    if observations.is_empty() {
        return Err(EventError::ZeroLength);
    }
    Ok(observations)
}

/// Predicted frequency of one harmonic at one heard time, hertz.
///
/// Implements `f_h(t_r) = h f0 c / (c + v_r(t_e))` from `docs/SDD.md` section
/// 4. The heard offset is converted to the reception offset from the emitted
/// closest approach with `u = (t_r − t0r) + d/c`, so the model evaluates the
/// light-time solution rather than a delayed copy of the received curve.
fn predicted_hz(
    parameters: &Parameters,
    observation: &Observation,
    sound_speed_m_per_s: f64,
) -> Option<f64> {
    if !parameters.is_physical(sound_speed_m_per_s) {
        return None;
    }
    let pass = StraightPass {
        closest_approach_m: parameters.closest_m,
        speed_m_per_s: parameters.speed_m_per_s,
    };
    let heard_offset_s = observation.heard_at_s - parameters.heard_at_s;
    let received_offset_s = pass
        .received_offset_from_heard_offset(heard_offset_s, sound_speed_m_per_s)
        .ok()?
        .0;
    let frequency = pass
        .received_frequency(
            Hertz(parameters.fundamental_hz),
            observation.harmonic,
            received_offset_s,
            sound_speed_m_per_s,
        )
        .ok()?
        .0;
    if frequency.is_finite() && frequency > 0.0 {
        Some(frequency)
    } else {
        None
    }
}

/// Huber weight: one inside the cut, `k / |r|` outside it.
fn huber_weight(residual: f64, cut: f64) -> f64 {
    let magnitude = residual.abs();
    if magnitude <= cut {
        1.0
    } else {
        cut / magnitude
    }
}

/// Huber objective: quadratic inside the cut, linear beyond it.
fn huber_cost(residual: f64, cut: f64) -> f64 {
    let magnitude = residual.abs();
    if magnitude <= cut {
        0.5 * residual * residual
    } else {
        cut * (magnitude - 0.5 * cut)
    }
}

/// Residuals, IRLS weights and objectives at one parameter vector.
///
/// `frozen_weights` is used by the profile-likelihood step: the robust weights
/// are held at their optimum values so the profiled statistic behaves like the
/// weighted `χ²` it is calibrated against.
fn evaluate(
    parameters: &Parameters,
    observations: &[Observation],
    config: &FitConfig,
    frozen_weights: Option<&[f64]>,
) -> Result<Evaluation, EventError> {
    let mut residuals = Vec::with_capacity(observations.len());
    let mut weights = Vec::with_capacity(observations.len());
    for (index, observation) in observations.iter().enumerate() {
        let predicted = predicted_hz(parameters, observation, config.sound_speed_m_per_s)
            .ok_or(EventError::ModelUndefined)?;
        let variance = observation.variance_hz2;
        if !variance.is_finite() || variance <= 0.0 {
            return Err(EventError::InvalidVariance(variance));
        }
        let residual = (observation.frequency_hz - predicted) / variance.sqrt();
        if !residual.is_finite() {
            return Err(EventError::NotFinite);
        }
        let weight = match frozen_weights {
            Some(frozen) => {
                let Some(&weight) = frozen.get(index) else {
                    return Err(EventError::LengthMismatch {
                        expected: observations.len(),
                        actual: frozen.len(),
                    });
                };
                weight
            }
            None => huber_weight(residual, config.huber_k),
        };
        residuals.push(residual);
        weights.push(weight);
    }

    let mut cost = 0.0_f64;
    let mut chi_squared = 0.0_f64;
    for (&residual, &weight) in residuals.iter().zip(weights.iter()) {
        let weighted_square = weight * residual * residual;
        chi_squared += weighted_square;
        cost += if frozen_weights.is_some() {
            weighted_square
        } else {
            huber_cost(residual, config.huber_k)
        };
    }
    if !cost.is_finite() || !chi_squared.is_finite() {
        return Err(EventError::NotFinite);
    }
    Ok(Evaluation {
        cost,
        chi_squared,
        residuals,
        weights,
    })
}

/// Step used for one parameter's central difference.
fn parameter_step(parameters: &Parameters, index: usize) -> f64 {
    match index {
        PARAM_FUNDAMENTAL => (parameters.fundamental_hz.abs() * RELATIVE_STEP).max(1.0e-9),
        PARAM_SPEED => (parameters.speed_m_per_s.abs() * RELATIVE_STEP).max(1.0e-9),
        PARAM_CLOSEST => (parameters.closest_m.abs() * RELATIVE_STEP).max(1.0e-9),
        _ => TIME_STEP_S,
    }
}

/// Numerical Jacobian of the predicted frequencies, one row per observation.
///
/// Central differences match the Cramér–Rao bound's gradient convention. Near
/// a physical bound one side is unavailable, so the available one-sided
/// difference is used.
fn jacobian_rows(
    parameters: &Parameters,
    observations: &[Observation],
    config: &FitConfig,
) -> Result<Vec<[f64; PARAMETER_COUNT]>, EventError> {
    let sound_speed = config.sound_speed_m_per_s;
    let mut rows = Vec::with_capacity(observations.len());
    for observation in observations {
        let base =
            predicted_hz(parameters, observation, sound_speed).ok_or(EventError::ModelUndefined)?;
        let mut row = [0.0_f64; PARAMETER_COUNT];
        for index in 0..PARAMETER_COUNT {
            let step = parameter_step(parameters, index);
            let forward = predicted_hz(
                &parameters.with_added(index, step),
                observation,
                sound_speed,
            );
            let backward = predicted_hz(
                &parameters.with_added(index, -step),
                observation,
                sound_speed,
            );
            let derivative = match (forward, backward) {
                (Some(forward), Some(backward)) => (forward - backward) / (2.0 * step),
                (Some(forward), None) => (forward - base) / step,
                (None, Some(backward)) => (base - backward) / step,
                (None, None) => 0.0,
            };
            if let Some(slot) = row.get_mut(index) {
                *slot = derivative;
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Weighted information matrix `JᵀWJ` and the Jacobian used to build it.
fn information_matrix(
    parameters: &Parameters,
    observations: &[Observation],
    config: &FitConfig,
    weights: &[f64],
) -> Result<
    (
        SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>,
        Vec<[f64; PARAMETER_COUNT]>,
    ),
    EventError,
> {
    let jacobian = jacobian_rows(parameters, observations, config)?;
    let mut information = SMatrix::<f64, PARAMETER_COUNT, PARAMETER_COUNT>::zeros();
    for (index, observation) in observations.iter().enumerate() {
        let Some(&row) = jacobian.get(index) else {
            return Err(EventError::LengthMismatch {
                expected: observations.len(),
                actual: jacobian.len(),
            });
        };
        let Some(&weight) = weights.get(index) else {
            return Err(EventError::LengthMismatch {
                expected: observations.len(),
                actual: weights.len(),
            });
        };
        let column = SVector::<f64, PARAMETER_COUNT>::from(row);
        information += column * column.transpose() * (weight / observation.variance_hz2);
    }
    Ok((information, jacobian))
}

/// Inputs shared by every objective evaluation in one fit.
struct FitProblem<'a> {
    observations: &'a [Observation],
    config: &'a FitConfig,
    frozen_weights: Option<&'a [f64]>,
}

/// The linearized normal equations at one point.
#[derive(Clone, Copy)]
struct NormalEquations {
    information: SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>,
    gradient: SVector<f64, PARAMETER_COUNT>,
}

/// Build `JᵀWJ` and `JᵀWr` at the current parameters.
///
/// A fixed parameter gets a zero row and column and a unit diagonal, so the
/// profile step leaves it exactly where it was pinned.
fn normal_equations(
    parameters: Parameters,
    current: &Evaluation,
    problem: &FitProblem<'_>,
    fixed: Option<(usize, f64)>,
) -> Result<NormalEquations, EventError> {
    let jacobian = jacobian_rows(&parameters, problem.observations, problem.config)?;
    let mut information = SMatrix::<f64, PARAMETER_COUNT, PARAMETER_COUNT>::zeros();
    let mut gradient = SVector::<f64, PARAMETER_COUNT>::zeros();
    for (index, observation) in problem.observations.iter().enumerate() {
        let Some(&row) = jacobian.get(index) else {
            return Err(EventError::LengthMismatch {
                expected: problem.observations.len(),
                actual: jacobian.len(),
            });
        };
        let Some(&residual) = current.residuals.get(index) else {
            return Err(EventError::LengthMismatch {
                expected: problem.observations.len(),
                actual: current.residuals.len(),
            });
        };
        let Some(&weight) = current.weights.get(index) else {
            return Err(EventError::LengthMismatch {
                expected: problem.observations.len(),
                actual: current.weights.len(),
            });
        };
        let column = SVector::<f64, PARAMETER_COUNT>::from(row);
        let scale = weight / observation.variance_hz2;
        information += column * column.transpose() * scale;
        gradient += column * (scale * residual);
    }

    if let Some((index, _)) = fixed {
        for other in 0..PARAMETER_COUNT {
            if let Some(slot) = information.get_mut((index, other)) {
                *slot = 0.0;
            }
            if let Some(slot) = information.get_mut((other, index)) {
                *slot = 0.0;
            }
        }
        if let Some(slot) = information.get_mut((index, index)) {
            *slot = 1.0;
        }
        if let Some(slot) = gradient.get_mut(index) {
            *slot = 0.0;
        }
    }
    Ok(NormalEquations {
        information,
        gradient,
    })
}

/// Try one damped step with backtracking.
///
/// The normal equations are often right about the direction but too
/// optimistic about how far the curve can be trusted. Halving the step before
/// changing the damping keeps the direction that the Jacobian chose.
fn try_damped_step(
    parameters: Parameters,
    current_cost: f64,
    normal: &NormalEquations,
    lambda: f64,
    fixed: Option<(usize, f64)>,
    problem: &FitProblem<'_>,
) -> Option<(Parameters, Evaluation, f64)> {
    let mut damped = normal.information;
    for index in 0..PARAMETER_COUNT {
        let diagonal = damped.get((index, index)).copied().unwrap_or(0.0);
        let scale = if diagonal.is_finite() && diagonal > 0.0 {
            diagonal
        } else {
            1.0
        };
        if let Some(slot) = damped.get_mut((index, index)) {
            *slot += lambda * scale;
        }
    }
    let delta = damped.lu().solve(&normal.gradient)?;

    let mut scale = 1.0_f64;
    for _ in 0..MAX_BACKTRACK_STEPS {
        let mut trial = parameters;
        let mut relative_step = 0.0_f64;
        let mut usable = true;
        for index in 0..PARAMETER_COUNT {
            if matches!(fixed, Some((fixed_index, _)) if fixed_index == index) {
                continue;
            }
            let step = scale * delta.get(index).copied().unwrap_or(0.0);
            if !step.is_finite() {
                usable = false;
                break;
            }
            let base = parameters.get(index).abs().max(1.0);
            relative_step = relative_step.max(step.abs() / base);
            trial.add(index, step);
        }
        if let Some((index, value)) = fixed {
            trial.set(index, value);
        }
        if !usable || !trial.is_physical(problem.config.sound_speed_m_per_s) {
            scale *= 0.5;
            continue;
        }
        let Ok(evaluation) = evaluate(
            &trial,
            problem.observations,
            problem.config,
            problem.frozen_weights,
        ) else {
            scale *= 0.5;
            continue;
        };
        if evaluation.cost < current_cost {
            return Some((trial, evaluation, relative_step));
        }
        scale *= 0.5;
    }
    None
}

/// Levenberg–Marquardt minimization of the robust objective.
///
/// `fixed` pins one parameter for the profile-likelihood step. The normal
/// equations use the residual weights at the current point, which is the
/// iteratively reweighted least-squares form of the Huber loss.
fn optimize(
    initial: Parameters,
    observations: &[Observation],
    config: &FitConfig,
    fixed: Option<(usize, f64)>,
    frozen_weights: Option<&[f64]>,
) -> Result<Optimum, EventError> {
    let problem = FitProblem {
        observations,
        config,
        frozen_weights,
    };
    let mut parameters = initial;
    if let Some((index, value)) = fixed {
        parameters.set(index, value);
    }
    if !parameters.is_physical(config.sound_speed_m_per_s) {
        return Err(EventError::ModelUndefined);
    }
    let mut current = evaluate(&parameters, observations, config, frozen_weights)?;
    let mut best = Optimum {
        parameters,
        cost: current.cost,
        iterations: 0,
    };
    // A profile run freezes the weights at the optimum; only the remaining
    // parameters move, so the profiled statistic keeps its χ² calibration.
    let mut lambda = config.lambda_initial;

    for iteration in 0..config.max_iterations {
        let normal = normal_equations(parameters, &current, &problem, fixed)?;
        let mut accepted = false;
        let mut accepted_relative_step = 0.0_f64;
        for _ in 0..MAX_INNER_STEPS {
            let Some((trial, evaluation, relative_step)) =
                try_damped_step(parameters, current.cost, &normal, lambda, fixed, &problem)
            else {
                lambda *= config.lambda_factor;
                if lambda > MAX_LAMBDA {
                    break;
                }
                continue;
            };
            parameters = trial;
            current = evaluation;
            best = Optimum {
                parameters,
                cost: current.cost,
                iterations: iteration.saturating_add(1),
            };
            accepted_relative_step = relative_step;
            accepted = true;
            lambda = (lambda / config.lambda_factor).max(MIN_LAMBDA);
            break;
        }

        if !accepted || accepted_relative_step <= config.step_tolerance {
            break;
        }
    }
    Ok(best)
}

/// One parameter's profile-likelihood problem.
struct ProfileContext<'a> {
    index: usize,
    parameters: &'a Parameters,
    weights: &'a [f64],
    observations: &'a [Observation],
    config: &'a FitConfig,
}

impl ProfileContext<'_> {
    /// `χ²` with this parameter pinned, after re-optimizing the other three.
    fn cost(&self, value: f64) -> Option<f64> {
        optimize(
            *self.parameters,
            self.observations,
            self.config,
            Some((self.index, value)),
            Some(self.weights),
        )
        .ok()
        .map(|optimum| optimum.cost)
    }

    /// Initial outward step, in the parameter's own unit.
    fn step(&self) -> f64 {
        let value = self.parameters.get(self.index);
        match self.index {
            PARAM_SPEED => (value.abs() * 0.05).max(0.5),
            PARAM_CLOSEST => (value.abs() * 0.05).max(5.0),
            _ => (value.abs() * 0.05).max(1.0e-3),
        }
    }

    /// Whether a profiled value is still inside the physical domain.
    fn in_bounds(&self, value: f64) -> bool {
        match self.index {
            PARAM_SPEED => {
                value.is_finite() && value > 0.0 && value < self.config.sound_speed_m_per_s
            }
            PARAM_CLOSEST => value.is_finite() && value > 0.0,
            _ => value.is_finite(),
        }
    }
}

/// Profile-likelihood interval for one parameter.
///
/// The other three parameters are re-optimized at every fixed value with the
/// robust weights frozen at the optimum. The weighted `χ²` therefore has its
/// usual one-degree-of-freedom calibration.
fn profile_interval(
    index: usize,
    parameters: &Parameters,
    weights: &[f64],
    observations: &[Observation],
    config: &FitConfig,
) -> Option<ProfileInterval> {
    let baseline = evaluate(parameters, observations, config, Some(weights))
        .ok()?
        .cost;
    let target = baseline + config.profile_delta_chi_squared;
    let context = ProfileContext {
        index,
        parameters,
        weights,
        observations,
        config,
    };
    let upper = profile_boundary(&context, 1.0, target);
    let lower = profile_boundary(&context, -1.0, target);
    if upper.is_none() && lower.is_none() {
        return None;
    }
    Some(ProfileInterval {
        lower,
        upper,
        delta_chi_squared: config.profile_delta_chi_squared,
    })
}

/// March outward from the optimum until the profile crosses its threshold.
fn profile_boundary(context: &ProfileContext<'_>, direction: f64, target: f64) -> Option<f64> {
    let optimum = context.parameters.get(context.index);
    let mut inside = optimum;
    let mut step = context.step();
    for _ in 0..MAX_PROFILE_STEPS {
        let candidate = inside + direction * step;
        if !context.in_bounds(candidate) {
            return None;
        }
        let cost = context.cost(candidate)?;
        if cost >= target {
            return bisect_profile(context, inside, candidate, target);
        }
        inside = candidate;
        step *= 2.0;
        if !step.is_finite() {
            return None;
        }
    }
    None
}

/// Refine a bracketed profile crossing. `inside` is below the threshold and
/// `outside` is at or above it; the returned value is the wider end.
fn bisect_profile(
    context: &ProfileContext<'_>,
    inside: f64,
    outside: f64,
    target: f64,
) -> Option<f64> {
    let mut low = inside;
    let mut high = outside;
    for _ in 0..PROFILE_BISECTIONS {
        let middle = f64::midpoint(low, high);
        let cost = context.cost(middle)?;
        if cost >= target {
            high = middle;
        } else {
            low = middle;
        }
        let span = (high - low).abs();
        let scale = high.abs().max(low.abs()).max(1.0);
        if span <= context.config.step_tolerance * scale {
            break;
        }
    }
    Some(high)
}

/// Speed/closest correlation from the scaled covariance.
fn correlation(
    covariance: &SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>,
) -> Result<f64, EventError> {
    for index in 0..PARAMETER_COUNT {
        let diagonal = covariance.get((index, index)).copied().unwrap_or(f64::NAN);
        if !diagonal.is_finite() || diagonal <= 0.0 {
            return Err(EventError::SingularFit);
        }
    }
    let speed_variance = covariance
        .get((PARAM_SPEED, PARAM_SPEED))
        .copied()
        .unwrap_or(f64::NAN);
    let closest_variance = covariance
        .get((PARAM_CLOSEST, PARAM_CLOSEST))
        .copied()
        .unwrap_or(f64::NAN);
    let cross = covariance
        .get((PARAM_SPEED, PARAM_CLOSEST))
        .copied()
        .unwrap_or(f64::NAN);
    let denominator = (speed_variance * closest_variance).sqrt();
    if !denominator.is_finite() || denominator <= 0.0 {
        return Err(EventError::SingularFit);
    }
    let value = (cross / denominator).clamp(-1.0, 1.0);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(EventError::NotFinite)
    }
}

/// Convert the nalgebra covariance into the public fixed array.
fn covariance_array(covariance: &SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>) -> [[f64; 4]; 4] {
    let mut output = [[0.0_f64; 4]; 4];
    for row in 0..PARAMETER_COUNT {
        for column in 0..PARAMETER_COUNT {
            let forward = covariance.get((row, column)).copied().unwrap_or(f64::NAN);
            let backward = covariance.get((column, row)).copied().unwrap_or(f64::NAN);
            let value = f64::midpoint(forward, backward);
            if let Some(slot) = output.get_mut(row).and_then(|line| line.get_mut(column)) {
                *slot = value;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        FitConfig, FitMethod, HarmonicTrack, TrackPoint, fit_event, fit_event_from_seeds,
        multi_start_seeds,
    };
    use crate::detect::EventHypothesis;
    use crate::error::EventError;
    use acoustics::StraightPass;
    use core_types::{AudioSeconds, EventId, Hertz};

    const SOUND_SPEED: f64 = 343.0;
    const HEARD_AT: f64 = 5.0;

    fn truth() -> (f64, f64, f64, f64) {
        (200.0, 80.0, 500.0, HEARD_AT)
    }

    fn config() -> FitConfig {
        FitConfig {
            sound_speed_m_per_s: SOUND_SPEED,
            // Normal fits do not need profiles; the profile test lowers this.
            profile_correlation_threshold: 1.0,
            ..FitConfig::default()
        }
    }

    fn event_id() -> EventId {
        EventId::new_v7()
    }

    /// Deterministic uniform value in `[0, 1)`.
    fn uniform(state: &mut u32) -> f64 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        f64::from(*state) / f64::from(u32::MAX)
    }

    /// Deterministic roughly Gaussian value with unit variance.
    fn gaussian(state: &mut u32) -> f64 {
        (0..12).map(|_| uniform(state)).sum::<f64>() - 6.0
    }

    /// Frequency of a harmonic at one heard time, hertz.
    fn model_hz(
        fundamental: f64,
        speed: f64,
        closest: f64,
        heard_at: f64,
        harmonic: u32,
        at: f64,
    ) -> f64 {
        let pass = StraightPass {
            closest_approach_m: closest,
            speed_m_per_s: speed,
        };
        let received = pass
            .received_offset_from_heard_offset(at - heard_at, SOUND_SPEED)
            .expect("valid geometry")
            .0;
        pass.received_frequency(Hertz(fundamental), harmonic, received, SOUND_SPEED)
            .expect("valid geometry")
            .0
    }

    /// A track on the true model with seeded Gaussian frequency noise.
    fn make_track(harmonic: u32, frames: usize, sigma_hz: f64, state: &mut u32) -> HarmonicTrack {
        let (fundamental, speed, closest, heard_at) = truth();
        let step_s = 10.0 / frames as f64;
        let mut points = Vec::with_capacity(frames);
        for frame in 0..frames {
            let at = frame as f64 * step_s;
            let clean = model_hz(fundamental, speed, closest, heard_at, harmonic, at);
            let noisy = clean + sigma_hz * gaussian(state);
            points.push(
                TrackPoint::new(AudioSeconds(at), Hertz(noisy), sigma_hz * sigma_hz)
                    .expect("valid point"),
            );
        }
        HarmonicTrack::new(harmonic, points).expect("valid track")
    }

    fn detector_seed() -> EventHypothesis {
        EventHypothesis {
            heard_at: AudioSeconds(4.8),
            fundamental_hz: Hertz(190.0),
            speed_m_per_s: 70.0,
            closest_m: 600.0,
        }
    }

    #[test]
    fn fr_dm_02_recovers_a_known_pass_with_its_covariance() {
        let mut state = 0x5eed_1234_u32;
        let tracks = vec![
            make_track(1, 41, 0.05, &mut state),
            make_track(2, 41, 0.05, &mut state),
        ];
        let report = fit_event(&tracks, event_id(), &detector_seed(), &config()).expect("fit");

        let (fundamental, speed, closest, heard_at) = truth();
        assert_eq!(report.method, FitMethod::TrackThenFit);
        assert_eq!(report.observations, 82);
        assert_eq!(report.harmonics, vec![1, 2]);
        assert!(report.iterations > 0);
        assert!(report.degrees_of_freedom >= 1);
        assert!((report.fit.fundamental_hz.0 - fundamental).abs() < 0.5);
        assert!((report.fit.speed_m_per_s - speed).abs() < 2.0);
        assert!((report.fit.closest_m.0 - closest).abs() < 15.0);
        assert!((report.fit.heard_at.0 - heard_at).abs() < 0.02);

        let fit = report.fit;
        for index in 0..4 {
            let variance = fit.covariance[index][index];
            assert!(
                variance.is_finite() && variance > 0.0,
                "variance {index} is {variance}"
            );
        }
        let sigma = [
            fit.covariance[0][0].sqrt(),
            fit.covariance[1][1].sqrt(),
            fit.covariance[2][2].sqrt(),
            fit.covariance[3][3].sqrt(),
        ];
        assert!((fit.fundamental_hz.0 - fundamental).abs() < 4.0 * sigma[0]);
        assert!((fit.speed_m_per_s - speed).abs() < 4.0 * sigma[1]);
        assert!((fit.closest_m.0 - closest).abs() < 4.0 * sigma[2]);
        assert!((fit.heard_at.0 - heard_at).abs() < 4.0 * sigma[3]);
    }

    #[test]
    fn fr_dm_04_joint_harmonics_tighten_the_fundamental() {
        let mut state = 0xabc_1234_u32;
        let fundamental_track = make_track(1, 41, 0.05, &mut state);
        let second_track = make_track(2, 41, 0.05, &mut state);
        let seed = detector_seed();

        let single = fit_event(
            std::slice::from_ref(&fundamental_track),
            event_id(),
            &seed,
            &config(),
        )
        .expect("single harmonic fit");
        let joint = fit_event(
            &[fundamental_track, second_track],
            event_id(),
            &seed,
            &config(),
        )
        .expect("joint fit");

        let single_sigma = single.fit.covariance[0][0].sqrt();
        let joint_sigma = joint.fit.covariance[0][0].sqrt();
        assert_eq!(single.harmonics, vec![1]);
        assert_eq!(joint.harmonics, vec![1, 2]);
        assert!(
            joint_sigma < single_sigma,
            "joint sigma {joint_sigma} should beat single {single_sigma}"
        );
    }

    #[test]
    fn fr_dm_02_robust_loss_resists_a_single_outlier() {
        let mut state = 0x1234_5678_u32;
        let mut track = make_track(1, 41, 0.05, &mut state);
        let Some(outlier) = track.points.get_mut(20) else {
            panic!("fixture has a middle point");
        };
        outlier.frequency_hz = Hertz(outlier.frequency_hz.0 + 40.0);

        let robust = fit_event(
            std::slice::from_ref(&track),
            event_id(),
            &detector_seed(),
            &config(),
        )
        .expect("robust fit");
        let non_robust_config = FitConfig {
            huber_k: 1.0e9,
            ..config()
        };
        let non_robust = fit_event(
            std::slice::from_ref(&track),
            event_id(),
            &detector_seed(),
            &non_robust_config,
        )
        .expect("non-robust fit");

        let (_, speed, closest, heard_at) = truth();
        let robust_error = (robust.fit.closest_m.0 - closest).abs()
            + (robust.fit.speed_m_per_s - speed).abs()
            + (robust.fit.heard_at.0 - heard_at).abs();
        let non_robust_error = (non_robust.fit.closest_m.0 - closest).abs()
            + (non_robust.fit.speed_m_per_s - speed).abs()
            + (non_robust.fit.heard_at.0 - heard_at).abs();
        assert!(
            robust_error < non_robust_error,
            "robust error {robust_error} should beat {non_robust_error}"
        );
    }

    #[test]
    fn fr_dm_02_covariance_is_inflated_when_residuals_exceed_the_stated_noise() {
        // The observations carry 1 Hz noise but claim 0.1 Hz, so the reduced
        // chi-square is far above one and must inflate the covariance.
        let mut state = 0xdead_beef_u32;
        let track = make_track(1, 41, 1.0, &mut state);
        let mut understated = track;
        for point in &mut understated.points {
            point.frequency_variance_hz2 = 0.01;
        }
        let report = fit_event(
            std::slice::from_ref(&understated),
            event_id(),
            &detector_seed(),
            &config(),
        )
        .expect("fit");
        assert!(
            report.covariance_scale > 10.0,
            "scale {} should exceed 10",
            report.covariance_scale
        );
    }

    #[test]
    fn fr_dm_02_profile_intervals_bracket_the_estimate() {
        let mut state = 0x0bad_f00d_u32;
        let tracks = vec![
            make_track(1, 41, 0.05, &mut state),
            make_track(2, 41, 0.05, &mut state),
        ];
        let profile_config = FitConfig {
            profile_correlation_threshold: 0.0,
            ..config()
        };
        let report =
            fit_event(&tracks, event_id(), &detector_seed(), &profile_config).expect("fit");
        assert!(report.speed_closest_correlation.abs() >= 0.0);
        let speed = report.speed_interval.expect("speed profile");
        let closest = report.closest_interval.expect("closest profile");
        assert!(speed.delta_chi_squared > 0.0);
        assert!(closest.delta_chi_squared > 0.0);
        let speed_low = speed.lower.expect("speed lower");
        let speed_high = speed.upper.expect("speed upper");
        assert!(speed_low <= report.fit.speed_m_per_s);
        assert!(report.fit.speed_m_per_s <= speed_high);
        let closest_low = closest.lower.expect("closest lower");
        let closest_high = closest.upper.expect("closest upper");
        assert!(closest_low <= report.fit.closest_m.0);
        assert!(report.fit.closest_m.0 <= closest_high);
        assert!(speed_low.is_finite() && speed_high.is_finite());
        assert!(closest_low.is_finite() && closest_high.is_finite());
    }

    #[test]
    fn fr_dm_02_multi_start_recovers_from_a_poor_detector_seed() {
        let mut state = 0x7777_8888_u32;
        let tracks = vec![make_track(1, 41, 0.05, &mut state)];
        let poor = EventHypothesis {
            heard_at: AudioSeconds(4.4),
            fundamental_hz: Hertz(180.0),
            speed_m_per_s: 125.0,
            closest_m: 1_100.0,
        };
        let seeds = multi_start_seeds(&poor, &config());
        assert!(seeds.len() > 1);
        let report = fit_event_from_seeds(&tracks, event_id(), &seeds, &config()).expect("fit");
        let (_, speed, closest, heard_at) = truth();
        eprintln!(
            "poor-seed fit v={} d={} t={}",
            report.fit.speed_m_per_s, report.fit.closest_m.0, report.fit.heard_at.0
        );
        assert!((report.fit.speed_m_per_s - speed).abs() < 3.0);
        assert!((report.fit.closest_m.0 - closest).abs() < 20.0);
        assert!((report.fit.heard_at.0 - heard_at).abs() < 0.03);
    }

    #[test]
    fn fr_dm_02_the_same_input_gives_the_same_fit() {
        let mut state = 0x0123_4567_u32;
        let tracks = vec![make_track(1, 41, 0.05, &mut state)];
        let id = event_id();
        let config = config();
        let first = fit_event(&tracks, id, &detector_seed(), &config).expect("first");
        let second = fit_event(&tracks, id, &detector_seed(), &config).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn fit_rejects_invalid_inputs() {
        let config = config();
        let seed = detector_seed();
        assert!(matches!(
            fit_event(&[], event_id(), &seed, &config),
            Err(EventError::ZeroLength)
        ));

        let mut state = 0x42_u32;
        let track = make_track(1, 41, 0.05, &mut state);
        let duplicate = track.clone();
        assert!(matches!(
            fit_event(&[track.clone(), duplicate], event_id(), &seed, &config),
            Err(EventError::DuplicateHarmonic(1))
        ));

        let mut broken = track.clone();
        if let Some(point) = broken.points.first_mut() {
            point.frequency_variance_hz2 = 0.0;
        }
        assert!(matches!(
            fit_event(&[broken], event_id(), &seed, &config),
            Err(EventError::InvalidVariance(_))
        ));

        let short = HarmonicTrack {
            harmonic: 1,
            points: track.points.iter().take(2).copied().collect(),
        };
        assert!(matches!(
            fit_event(&[short], event_id(), &seed, &config),
            Err(EventError::InsufficientTrackPoints { .. })
        ));

        let bad_config = FitConfig {
            huber_k: 0.0,
            ..config
        };
        assert!(matches!(
            fit_event(std::slice::from_ref(&track), event_id(), &seed, &bad_config),
            Err(EventError::InvalidRate(_))
        ));
        assert!(matches!(
            fit_event_from_seeds(&[track], event_id(), &[], &config),
            Err(EventError::NoHypothesis)
        ));
    }

    #[test]
    fn harmonic_zero_is_rejected() {
        let point = TrackPoint::new(AudioSeconds(0.0), Hertz(200.0), 1.0).expect("valid point");
        assert!(matches!(
            HarmonicTrack::new(0, vec![point]),
            Err(EventError::InvalidHarmonic(0))
        ));
        assert!(matches!(
            TrackPoint::new(AudioSeconds(f64::NAN), Hertz(200.0), 1.0),
            Err(EventError::NotFinite)
        ));
        assert!(matches!(
            TrackPoint::new(AudioSeconds(0.0), Hertz(0.0), 1.0),
            Err(EventError::InvalidVariance(_))
        ));
    }

    #[test]
    fn from_ridge_converts_frame_times_and_variances() {
        use dsp::{Corridor, RidgeConfig, StftConfig, ZMap, track_ridge};

        let stft = StftConfig::survey(4_000.0).expect("survey config");
        let bins = stft.bin_count();
        let frames = 20;
        let mut values = vec![0.5_f32; frames * bins];
        for frame in 0..frames {
            let bin = 150 + frame;
            if let Some(slot) = values.get_mut(frame * bins + bin) {
                *slot = 20.0;
            }
        }
        let times: Vec<f64> = (0..frames).map(|frame| frame as f64 * 0.25).collect();
        let map = ZMap::from_parts(stft, times, values).expect("map");
        let corridor = Corridor::new(vec![(100, 250); frames], map.bin_count()).expect("corridor");
        let ridge = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("ridge");
        let track = HarmonicTrack::from_ridge(1, &ridge, &map, 0.25).expect("track");
        assert_eq!(track.len(), frames);
        let width = map.config().bin_width_hz();
        for (frame, point) in track.points.iter().enumerate() {
            assert_eq!(point.heard_at, AudioSeconds(map.frame_times_s()[frame]));
            let expected = ridge.points()[frame].frequency_hz;
            assert!((point.frequency_hz.0 - expected).abs() < 1e-9);
            assert!((point.frequency_variance_hz2 - 0.25 * width * width).abs() < 1e-12);
        }
    }
}
