//! Direct parametric estimator on the whitened map (FR-DM-05).
//!
//! `docs/SDD.md` section 5.5 defines a second estimator that shares the
//! emission-time model of section 4 with the track-then-fit path. It maximises
//! the whitened power sampled along the model curve over all requested
//! harmonics and frames, starting from a detector or track-fit seed. No ridge
//! is needed, so the estimator can measure faint passes where tracking fails.
//!
//! The map stores z values, the normalized magnitude. At each frame the model
//! frequency falls between two bins; the z values there are interpolated
//! linearly and squared, which is the whitened power. Under Gaussian noise `z`
//! is standard normal, so `z²` is exponential and the sum is a generalized
//! likelihood ratio under exponential noise. The covariance is the inverse of
//! the negative Hessian of that sum at the optimum.
//!
//! The defaults are `PROVISIONAL(G1)`: the simulator suite fixes the search
//! steps and the minimum sample count once direct fits can be scored on faint
//! events.

use core_types::{AudioSeconds, EventId, Hertz, Metres};
use dsp::ZMap;
use nalgebra::SMatrix;

use crate::detect::EventHypothesis;
use crate::error::EventError;
use crate::fit::{
    EventFit, FitMethod, FitReport, PARAMETER_COUNT, Parameters, correlation, covariance_array,
    predicted_hz_at,
};

/// Settings for the direct parametric estimator.
///
/// The defaults are `PROVISIONAL(G1)`: they are chosen to converge on the
/// simulator's faint passes and are replaced when the estimator suite can
/// measure coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectConfig {
    /// Speed of sound used by the emission-time model, metres per second.
    pub sound_speed_m_per_s: f64,
    /// Upper bound on simplex-search iterations.
    pub max_iterations: usize,
    /// Relative parameter step at which the search stops.
    pub step_tolerance: f64,
    /// Factor by which the simplex shrinks toward its best vertex.
    pub shrink_factor: f64,
    /// Samples the map and harmonic list must provide, more than the four
    /// free parameters.
    pub min_samples: usize,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            sound_speed_m_per_s: 343.0,
            max_iterations: 200,
            step_tolerance: 1.0e-4,
            shrink_factor: 0.5,
            min_samples: 12,
        }
    }
}

impl DirectConfig {
    /// Check the settings before a direct fit.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite or non-positive sound
    /// speed, [`EventError::ZeroLength`] for a zero iteration count,
    /// [`EventError::InvalidRate`] for a non-positive tolerance or a shrink
    /// factor outside `(0, 1)`, and [`EventError::InsufficientObservations`]
    /// for a sample floor that leaves no residual degrees of freedom.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.sound_speed_m_per_s.is_finite() || self.sound_speed_m_per_s <= 0.0 {
            return Err(EventError::NotFinite);
        }
        if self.max_iterations == 0 {
            return Err(EventError::ZeroLength);
        }
        if !self.step_tolerance.is_finite() || self.step_tolerance <= 0.0 {
            return Err(EventError::InvalidRate(self.step_tolerance));
        }
        if !self.shrink_factor.is_finite() || !(0.0..1.0).contains(&self.shrink_factor) {
            return Err(EventError::InvalidRate(self.shrink_factor));
        }
        if self.min_samples <= PARAMETER_COUNT {
            return Err(EventError::InsufficientObservations {
                needed: PARAMETER_COUNT + 1,
                actual: self.min_samples,
            });
        }
        Ok(())
    }
}

/// Fit one event directly on the whitened map.
///
/// `seed` may come from the detector or from a previous track-then-fit result;
/// `harmonics` lists the visible harmonics, with one the fundamental. The
/// estimator maximizes the summed whitened power along the emission-time curve
/// and returns the same [`FitReport`] shape as the track fit, with
/// [`FitMethod::Direct`]. The covariance is `(-H)⁻¹` at the optimum.
///
/// # Errors
///
/// Returns [`EventError::EmptyMap`] for a map without frames or bins,
/// [`EventError::NotFinite`] for non-finite frame times,
/// [`EventError::ZeroLength`] for an empty harmonic list,
/// [`EventError::InvalidHarmonic`] for harmonic zero,
/// [`EventError::DuplicateHarmonic`] for a repeated harmonic,
/// [`EventError::InsufficientObservations`] when the map and harmonics leave
/// no residual degrees of freedom, [`EventError::NoHypothesis`] when the seed
/// is unphysical or the curve leaves the map, and [`EventError::SingularFit`]
/// when the Hessian cannot be inverted.
pub fn fit_event_direct(
    map: &ZMap,
    event_id: EventId,
    seed: &EventHypothesis,
    harmonics: &[u32],
    config: &DirectConfig,
) -> Result<FitReport, EventError> {
    config.validate()?;
    validate_map(map)?;
    let harmonics = sorted_harmonics(harmonics)?;
    let samples = map.frame_count().saturating_mul(harmonics.len());
    if samples < config.min_samples {
        return Err(EventError::InsufficientObservations {
            needed: config.min_samples,
            actual: samples,
        });
    }

    let problem = DirectProblem {
        map,
        harmonics: &harmonics,
        config,
        bins: map.bin_count(),
        width_hz: map.config().bin_width_hz(),
        times: map.frame_times_s(),
    };
    let start = Parameters::from_hypothesis(seed);
    if !start.is_physical(config.sound_speed_m_per_s) {
        return Err(EventError::NoHypothesis);
    }
    let start_value = problem.objective(&start).ok_or(EventError::NoHypothesis)?;
    let optimum = problem.optimize(start, start_value);
    let covariance = problem
        .covariance(&optimum.parameters)
        .ok_or(EventError::SingularFit)?;
    let speed_closest_correlation = correlation(&covariance)?;

    Ok(FitReport {
        fit: EventFit {
            event_id,
            fundamental_hz: Hertz(optimum.parameters.fundamental_hz),
            speed_m_per_s: optimum.parameters.speed_m_per_s,
            closest_m: Metres(optimum.parameters.closest_m),
            heard_at: AudioSeconds(optimum.parameters.heard_at_s),
            covariance: covariance_array(&covariance),
        },
        method: FitMethod::Direct,
        chi_squared: optimum.objective,
        degrees_of_freedom: samples.saturating_sub(PARAMETER_COUNT),
        covariance_scale: 1.0,
        robust_cost: -optimum.objective,
        speed_closest_correlation,
        speed_interval: None,
        closest_interval: None,
        iterations: optimum.iterations,
        observations: samples,
        harmonics,
    })
}

/// Reject a map with no usable geometry.
fn validate_map(map: &ZMap) -> Result<(), EventError> {
    if map.frame_count() == 0 || map.bin_count() == 0 {
        return Err(EventError::EmptyMap);
    }
    if !map.config().bin_width_hz().is_finite() || map.config().bin_width_hz() <= 0.0 {
        return Err(EventError::NotFinite);
    }
    if map.frame_times_s().iter().any(|time| !time.is_finite()) {
        return Err(EventError::NotFinite);
    }
    Ok(())
}

/// Validate and sort the harmonic list, rejecting zero and duplicates.
fn sorted_harmonics(harmonics: &[u32]) -> Result<Vec<u32>, EventError> {
    if harmonics.is_empty() {
        return Err(EventError::ZeroLength);
    }
    let mut sorted = Vec::with_capacity(harmonics.len());
    for &harmonic in harmonics {
        if harmonic == 0 {
            return Err(EventError::InvalidHarmonic(harmonic));
        }
        if sorted.contains(&harmonic) {
            return Err(EventError::DuplicateHarmonic(harmonic));
        }
        sorted.push(harmonic);
    }
    sorted.sort_unstable();
    Ok(sorted)
}

/// The best point found by the simplex search.
#[derive(Debug, Clone, Copy)]
struct Optimum {
    parameters: Parameters,
    objective: f64,
    iterations: usize,
}

/// One simplex vertex: parameters and their objective.
#[derive(Debug, Clone, Copy)]
struct Vertex {
    parameters: Parameters,
    value: f64,
}

/// Reflection coefficient of the Nelder–Mead simplex.
const SIMPLEX_REFLECTION: f64 = 1.0;
/// Expansion coefficient of the Nelder–Mead simplex.
const SIMPLEX_EXPANSION: f64 = 2.0;
/// Contraction coefficient of the Nelder–Mead simplex.
const SIMPLEX_CONTRACTION: f64 = 0.5;

/// One map, harmonic list and configuration, ready to score parameter vectors.
struct DirectProblem<'a> {
    map: &'a ZMap,
    harmonics: &'a [u32],
    config: &'a DirectConfig,
    bins: usize,
    width_hz: f64,
    times: &'a [f64],
}

impl DirectProblem<'_> {
    /// Whitened power at one frame and frequency.
    ///
    /// The z values of the two adjacent bins are interpolated linearly at the
    /// fractional bin coordinate and squared. Returns `None` outside the map so
    /// a curve that leaves the measured band is rejected rather than scored on
    /// a truncated set of samples.
    fn power_at(&self, frame: usize, frequency_hz: f64) -> Option<f64> {
        if !frequency_hz.is_finite() || frequency_hz <= 0.0 {
            return None;
        }
        let coordinate = frequency_hz / self.width_hz;
        let last_index = self.bins.checked_sub(1)?;
        let last = last_index as f64;
        if !coordinate.is_finite() || coordinate < 0.0 || coordinate > last {
            return None;
        }
        let lower = coordinate.floor();
        let fraction = coordinate - lower;
        let lower_index = lower as usize;
        let upper_index = lower_index.saturating_add(1).min(last_index);
        let lower_z = f64::from(self.map.value(frame, lower_index)?);
        let upper_z = f64::from(self.map.value(frame, upper_index)?);
        let z = lower_z + (upper_z - lower_z) * fraction;
        Some(z * z)
    }

    /// Summed whitened power along the model curve.
    fn objective(&self, parameters: &Parameters) -> Option<f64> {
        let mut total = 0.0_f64;
        for &harmonic in self.harmonics {
            for (frame, &time_s) in self.times.iter().enumerate() {
                let frequency_hz = predicted_hz_at(
                    parameters,
                    time_s,
                    harmonic,
                    self.config.sound_speed_m_per_s,
                )?;
                total += self.power_at(frame, frequency_hz)?;
            }
        }
        if total.is_finite() { Some(total) } else { None }
    }

    /// Typical frame spacing, used to scale the time step.
    fn frame_spacing(&self) -> f64 {
        match (self.times.first(), self.times.get(1)) {
            (Some(first), Some(second)) => {
                let spacing = (second - first).abs();
                if spacing.is_finite() && spacing > 0.0 {
                    spacing
                } else {
                    1.0e-3
                }
            }
            _ => 1.0e-3,
        }
    }

    /// Initial simplex edge lengths for `(f0, v, d, t0r)`.
    fn initial_steps(&self, parameters: &Parameters) -> [f64; PARAMETER_COUNT] {
        [
            (0.5 * self.width_hz).max(parameters.fundamental_hz.abs() * 1.0e-3),
            (parameters.speed_m_per_s.abs() * 0.05).max(0.1),
            (parameters.closest_m.abs() * 0.05).max(0.1),
            self.frame_spacing(),
        ]
    }

    /// Steps for the finite-difference Hessian.
    ///
    /// These are wider than the simplex tolerance so the curvature is measured
    /// over a fraction of a bin and a small fraction of each parameter, not
    /// over floating-point rounding.
    fn hessian_steps(&self, parameters: &Parameters) -> [f64; PARAMETER_COUNT] {
        [
            (0.05 * self.width_hz).max(parameters.fundamental_hz.abs() * 1.0e-4),
            (parameters.speed_m_per_s.abs() * 1.0e-2).max(1.0e-2),
            (parameters.closest_m.abs() * 1.0e-2).max(1.0e-2),
            (self.frame_spacing() * 1.0e-2).max(1.0e-6),
        ]
    }

    /// Maximize the objective with a Nelder–Mead simplex.
    ///
    /// The simplex is used rather than coordinate steps because speed and
    /// closest distance travel along a narrow correlated valley: a step in
    /// either alone can lower the power while the diagonal move raises it.
    fn optimize(&self, start: Parameters, start_value: f64) -> Optimum {
        let mut vertices = self.initial_simplex(start, start_value);
        let mut iterations = 0_usize;
        while iterations < self.config.max_iterations {
            vertices.sort_by(|left, right| right.value.total_cmp(&left.value));
            let Some(best) = vertices.first().copied() else {
                break;
            };
            if self.converged(&vertices, &best) {
                break;
            }
            iterations = iterations.saturating_add(1);
            self.refine(&mut vertices, &best);
        }
        vertices.sort_by(|left, right| right.value.total_cmp(&left.value));
        match vertices.first().copied() {
            Some(best) => Optimum {
                parameters: best.parameters,
                objective: best.value,
                iterations,
            },
            None => Optimum {
                parameters: start,
                objective: start_value,
                iterations,
            },
        }
    }

    /// Build the starting simplex, one vertex per parameter.
    fn initial_simplex(&self, start: Parameters, start_value: f64) -> Vec<Vertex> {
        let steps = self.initial_steps(&start);
        let mut vertices = Vec::with_capacity(PARAMETER_COUNT.saturating_add(1));
        vertices.push(Vertex {
            parameters: start,
            value: start_value,
        });
        for index in 0..PARAMETER_COUNT {
            let step = steps.get(index).copied().unwrap_or(0.0);
            let parameters = start.with_added(index, step);
            vertices.push(self.vertex(parameters));
        }
        vertices
    }

    /// Evaluate one point, marking unphysical or out-of-map points as worst.
    fn vertex(&self, parameters: Parameters) -> Vertex {
        let value = if parameters.is_physical(self.config.sound_speed_m_per_s) {
            self.objective(&parameters).unwrap_or(f64::NEG_INFINITY)
        } else {
            f64::NEG_INFINITY
        };
        Vertex { parameters, value }
    }

    /// One Nelder–Mead reflection, expansion, contraction or shrink.
    fn refine(&self, vertices: &mut [Vertex], best: &Vertex) {
        let Some(worst) = vertices.last().copied() else {
            return;
        };
        let Some(second_worst) = vertices
            .len()
            .checked_sub(2)
            .and_then(|index| vertices.get(index))
            .copied()
        else {
            return;
        };
        let Some(best_set) = vertices.get(..vertices.len().saturating_sub(1)) else {
            return;
        };
        let centroid = centroid(best_set);
        let reflected = self.vertex(move_toward(
            &centroid,
            &worst.parameters,
            -SIMPLEX_REFLECTION,
        ));
        let replacement = if reflected.value > best.value {
            let expanded = self.vertex(move_toward(
                &centroid,
                &worst.parameters,
                -SIMPLEX_EXPANSION,
            ));
            if expanded.value > reflected.value {
                expanded
            } else {
                reflected
            }
        } else if reflected.value > second_worst.value {
            reflected
        } else {
            let target = if reflected.value > worst.value {
                &reflected
            } else {
                &worst
            };
            let contracted = self.vertex(move_toward(
                &centroid,
                &target.parameters,
                SIMPLEX_CONTRACTION,
            ));
            if contracted.value > worst.value {
                contracted
            } else {
                self.shrink(vertices, best);
                return;
            }
        };
        if let Some(slot) = vertices.last_mut() {
            *slot = replacement;
        }
    }

    /// Pull every vertex except the best halfway toward it.
    fn shrink(&self, vertices: &mut [Vertex], best: &Vertex) {
        for vertex in vertices.iter_mut().skip(1) {
            let parameters = move_toward(
                &best.parameters,
                &vertex.parameters,
                self.config.shrink_factor,
            );
            *vertex = self.vertex(parameters);
        }
    }

    /// Whether every vertex lies within the tolerance of the best one.
    fn converged(&self, vertices: &[Vertex], best: &Vertex) -> bool {
        let tolerance = self.config.step_tolerance;
        vertices.iter().all(|vertex| {
            (0..PARAMETER_COUNT).all(|index| {
                let scale = best.parameters.get(index).abs().max(1.0);
                (vertex.parameters.get(index) - best.parameters.get(index)).abs()
                    <= scale * tolerance
            })
        })
    }

    /// Covariance from the negative Hessian at the optimum.
    fn covariance(
        &self,
        parameters: &Parameters,
    ) -> Option<SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>> {
        let hessian = self.hessian(parameters)?;
        let inverse = (-hessian).try_inverse()?;
        for index in 0..PARAMETER_COUNT {
            let diagonal = inverse.get((index, index)).copied()?;
            if !diagonal.is_finite() || diagonal <= 0.0 {
                return None;
            }
        }
        Some(inverse)
    }

    /// Central-difference Hessian of the summed power.
    fn hessian(
        &self,
        parameters: &Parameters,
    ) -> Option<SMatrix<f64, PARAMETER_COUNT, PARAMETER_COUNT>> {
        let steps = self.hessian_steps(parameters);
        let base = self.objective(parameters)?;
        let mut hessian = SMatrix::<f64, PARAMETER_COUNT, PARAMETER_COUNT>::zeros();
        for row in 0..PARAMETER_COUNT {
            let row_step = *steps.get(row)?;
            let plus = self.objective(&parameters.with_added(row, row_step))?;
            let minus = self.objective(&parameters.with_added(row, -row_step))?;
            let second = (plus - 2.0 * base + minus) / (row_step * row_step);
            if let Some(slot) = hessian.get_mut((row, row)) {
                *slot = second;
            }
            for column in (row + 1)..PARAMETER_COUNT {
                let column_step = *steps.get(column)?;
                let plus_plus = self.objective(
                    &parameters
                        .with_added(row, row_step)
                        .with_added(column, column_step),
                )?;
                let plus_minus = self.objective(
                    &parameters
                        .with_added(row, row_step)
                        .with_added(column, -column_step),
                )?;
                let minus_plus = self.objective(
                    &parameters
                        .with_added(row, -row_step)
                        .with_added(column, column_step),
                )?;
                let minus_minus = self.objective(
                    &parameters
                        .with_added(row, -row_step)
                        .with_added(column, -column_step),
                )?;
                let cross = (plus_plus - plus_minus - minus_plus + minus_minus)
                    / (4.0 * row_step * column_step);
                if let Some(slot) = hessian.get_mut((row, column)) {
                    *slot = cross;
                }
                if let Some(slot) = hessian.get_mut((column, row)) {
                    *slot = cross;
                }
            }
        }
        Some(hessian)
    }
}

/// Centroid of a non-empty vertex set.
fn centroid(vertices: &[Vertex]) -> Parameters {
    let Some(first) = vertices.first() else {
        return Parameters {
            fundamental_hz: 0.0,
            speed_m_per_s: 0.0,
            closest_m: 0.0,
            heard_at_s: 0.0,
        };
    };
    let mut result = first.parameters;
    let count = vertices.len() as f64;
    for index in 0..PARAMETER_COUNT {
        let sum: f64 = vertices
            .iter()
            .map(|vertex| vertex.parameters.get(index))
            .sum();
        result.set(index, sum / count);
    }
    result
}

/// Move `from` a fraction of the way toward `to`; negative factors extrapolate.
fn move_toward(from: &Parameters, to: &Parameters, factor: f64) -> Parameters {
    let mut result = *from;
    for index in 0..PARAMETER_COUNT {
        let delta = to.get(index) - from.get(index);
        result = result.with_added(index, factor * delta);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{DirectConfig, DirectProblem, fit_event_direct};
    use crate::detect::EventHypothesis;
    use crate::fit::{FitMethod, Parameters, predicted_hz_at};
    use core_types::{AudioSeconds, EventId, Hertz};
    use dsp::{StftConfig, ZMap};

    /// Speed of sound for every fixture, metres per second.
    const SOUND_SPEED: f64 = 343.0;
    /// Peak z of the injected ridge before noise.
    const RIDGE_AMPLITUDE: f64 = 1.5;
    /// Standard deviation of the injected ridge in frequency bins.
    const RIDGE_SIGMA_BINS: f64 = 1.5;
    /// Frames in the synthetic survey map.
    const FRAMES: usize = 250;
    /// Analysis sample rate, hertz.
    const SAMPLE_RATE_HZ: f64 = 4_000.0;
    /// STFT hop in samples.
    const HOP: usize = 64;

    fn event_id() -> EventId {
        EventId::new_v7()
    }

    fn truth() -> EventHypothesis {
        EventHypothesis {
            heard_at: AudioSeconds(2.0),
            fundamental_hz: Hertz(180.0),
            speed_m_per_s: 120.0,
            closest_m: 800.0,
        }
    }

    fn detector_seed() -> EventHypothesis {
        EventHypothesis {
            heard_at: AudioSeconds(2.03),
            fundamental_hz: Hertz(188.0),
            speed_m_per_s: 135.0,
            closest_m: 950.0,
        }
    }

    fn config() -> DirectConfig {
        DirectConfig {
            sound_speed_m_per_s: SOUND_SPEED,
            ..DirectConfig::default()
        }
    }

    fn stft() -> StftConfig {
        StftConfig::new(SAMPLE_RATE_HZ, 256, HOP).expect("survey geometry")
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

    /// A z-map with seeded noise and a faint emission-time ridge.
    fn faint_map(state: &mut u32) -> ZMap {
        let stft = stft();
        let bins = stft.bin_count();
        let width_hz = stft.bin_width_hz();
        let times: Vec<f64> = (0..FRAMES)
            .map(|frame| frame as f64 * HOP as f64 / SAMPLE_RATE_HZ)
            .collect();
        let truth = Parameters::from_hypothesis(&truth());
        let mut values = vec![0.0_f32; FRAMES * bins];
        for frame in 0..FRAMES {
            for bin in 0..bins {
                values[frame * bins + bin] = gaussian(state) as f32;
            }
            for &harmonic in &[1_u32, 2] {
                let time_s = times[frame];
                let frequency_hz = predicted_hz_at(&truth, time_s, harmonic, SOUND_SPEED)
                    .expect("fixture geometry");
                let centre = frequency_hz / width_hz;
                let first = ((centre - 4.0 * RIDGE_SIGMA_BINS).floor()).max(0.0) as usize;
                let last =
                    ((centre + 4.0 * RIDGE_SIGMA_BINS).ceil()).min((bins - 1) as f64) as usize;
                for bin in first..=last {
                    let offset = (bin as f64 - centre) / RIDGE_SIGMA_BINS;
                    let signal = RIDGE_AMPLITUDE * (-0.5 * offset * offset).exp();
                    values[frame * bins + bin] += signal as f32;
                }
            }
        }
        ZMap::from_parts(stft, times, values).expect("map shape")
    }

    fn relative_error(actual: f64, expected: f64) -> f64 {
        (actual - expected).abs() / expected.abs().max(1.0)
    }

    #[test]
    fn fr_dm_05_direct_recovers_a_faint_pass() {
        let mut state = 0x1234_5678_u32;
        let map = faint_map(&mut state);
        let seed = detector_seed();
        let report =
            fit_event_direct(&map, event_id(), &seed, &[1, 2], &config()).expect("direct fit");
        let truth = truth();
        let truth_parameters = Parameters::from_hypothesis(&truth);
        let fit_parameters = Parameters::from_hypothesis(&EventHypothesis {
            heard_at: report.fit.heard_at,
            fundamental_hz: report.fit.fundamental_hz,
            speed_m_per_s: report.fit.speed_m_per_s,
            closest_m: report.fit.closest_m.0,
        });

        assert_eq!(report.method, FitMethod::Direct);
        assert!(
            relative_error(report.fit.fundamental_hz.0, truth.fundamental_hz.0) < 0.03,
            "fundamental {} vs {}",
            report.fit.fundamental_hz.0,
            truth.fundamental_hz.0
        );
        assert!(
            (report.fit.heard_at.0 - truth.heard_at.0).abs() < 0.06,
            "heard {} vs {}",
            report.fit.heard_at.0,
            truth.heard_at.0
        );
        // Speed and closest distance trade off along a flat valley over this
        // short window, so the observable to check is the fitted curve. A
        // fifteenth of a bin is well below the injected ridge width of 1.5
        // bins and about the limit a faint pass supports.
        let tolerance_hz = 0.15 * map.config().bin_width_hz();
        for time_s in [0.0, 1.0, 2.0, 3.0, 4.0] {
            let expected =
                predicted_hz_at(&truth_parameters, time_s, 1, SOUND_SPEED).expect("truth curve");
            let actual =
                predicted_hz_at(&fit_parameters, time_s, 1, SOUND_SPEED).expect("fitted curve");
            assert!(
                (actual - expected).abs() < tolerance_hz,
                "curve at {time_s} s: {actual} vs {expected} Hz"
            );
        }
        // The valley is real: speed and distance remain in the detector's
        // neighbourhood even when their individual errors are large.
        assert!(
            relative_error(report.fit.speed_m_per_s, truth.speed_m_per_s) < 0.20,
            "speed {} vs {}",
            report.fit.speed_m_per_s,
            truth.speed_m_per_s
        );
        assert!(
            relative_error(report.fit.closest_m.0, truth.closest_m) < 0.35,
            "closest {} vs {}",
            report.fit.closest_m.0,
            truth.closest_m
        );
        assert!(report.chi_squared > 0.0);
        for index in 0..4 {
            let diagonal = report.fit.covariance[index][index];
            assert!(
                diagonal.is_finite() && diagonal > 0.0,
                "covariance diagonal {index} is {diagonal}"
            );
        }
    }

    #[test]
    fn fr_dm_05_objective_peaks_at_the_true_pass() {
        let mut state = 0x0bad_f00d_u32;
        let map = faint_map(&mut state);
        let harmonics = [1_u32, 2];
        let config = config();
        let problem = DirectProblem {
            map: &map,
            harmonics: &harmonics,
            config: &config,
            bins: map.bin_count(),
            width_hz: map.config().bin_width_hz(),
            times: map.frame_times_s(),
        };
        let truth_value = problem
            .objective(&Parameters::from_hypothesis(&truth()))
            .expect("truth inside map");
        let seed_value = problem
            .objective(&Parameters::from_hypothesis(&detector_seed()))
            .expect("seed inside map");
        assert!(truth_value > seed_value, "{truth_value} <= {seed_value}");
    }

    #[test]
    fn fr_dm_05_direct_is_deterministic() {
        let mut state = 0x00c0_ffee_u32;
        let map = faint_map(&mut state);
        let id = event_id();
        let first =
            fit_event_direct(&map, id, &detector_seed(), &[1, 2], &config()).expect("first fit");
        let second =
            fit_event_direct(&map, id, &detector_seed(), &[1, 2], &config()).expect("second fit");
        assert_eq!(first.fit, second.fit);
        assert_eq!(first.iterations, second.iterations);
        assert_eq!(first.chi_squared.to_bits(), second.chi_squared.to_bits());
    }

    #[test]
    fn fr_dm_05_direct_rejects_invalid_inputs() {
        let mut state = 0x5eed_1234_u32;
        let map = faint_map(&mut state);
        let seed = detector_seed();
        let config = config();

        assert!(matches!(
            fit_event_direct(&map, event_id(), &seed, &[], &config),
            Err(crate::EventError::ZeroLength)
        ));
        assert!(matches!(
            fit_event_direct(&map, event_id(), &seed, &[0], &config),
            Err(crate::EventError::InvalidHarmonic(0))
        ));
        assert!(matches!(
            fit_event_direct(&map, event_id(), &seed, &[1, 1], &config),
            Err(crate::EventError::DuplicateHarmonic(1))
        ));

        let mut bad_config = config;
        bad_config.min_samples = 4;
        assert!(matches!(
            fit_event_direct(&map, event_id(), &seed, &[1], &bad_config),
            Err(crate::EventError::InsufficientObservations { .. })
        ));

        let mut supersonic = seed;
        supersonic.speed_m_per_s = SOUND_SPEED;
        assert!(matches!(
            fit_event_direct(&map, event_id(), &supersonic, &[1, 2], &config),
            Err(crate::EventError::NoHypothesis)
        ));

        let stft = stft();
        let empty = ZMap::from_parts(stft, Vec::new(), Vec::new()).expect("empty map");
        assert!(matches!(
            fit_event_direct(&empty, event_id(), &seed, &[1], &config),
            Err(crate::EventError::EmptyMap)
        ));
    }
}
