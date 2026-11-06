//! Ridge tracking ladder for a whitened spectrogram (SDD section 5).
//!
//! Three levels, each used only when the one below is too noisy:
//!
//! 1. the per-frame peak inside a template corridor;
//! 2. a Viterbi path through the z-map with a smoothness cost on frequency
//!    change;
//! 3. a constant-velocity Kalman filter with a Rauch-Tung-Striebel smoother,
//!    which returns a per-frame variance.
//!
//! The corridor is the detector's job: it supplies one allowed bin range per
//! frame, following the predicted Doppler curve.

use crate::error::DspError;
use crate::whiten::ZMap;

/// A priori state and covariance at one frame: `(bin, bin per frame, P00, P01,
/// P11)`.
type PriorEstimate = (f64, f64, f64, f64, f64);

/// Filtered state and covariance at one frame: `(bin, bin per frame,
/// [P00, P01, P11])`.
type FilteredEstimate = (f64, f64, [f64; 3]);

/// Level used to produce a ridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RidgeMethod {
    /// Per-frame peak inside the corridor.
    Peaks,
    /// Viterbi path with a smoothness cost.
    Viterbi,
    /// Kalman filter with an RTS smoother.
    Smoothed,
}

/// One tracked point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RidgePoint {
    /// Tracked bin, fractional for a smoothed ridge.
    pub bin: f64,
    /// Frequency of that bin position, in hertz.
    pub frequency_hz: f64,
    /// z value at the rounded bin, from the whitened map.
    pub z: f64,
}

/// A tracked ridge, one point per frame.
#[derive(Debug, Clone)]
pub struct Ridge {
    method: RidgeMethod,
    points: Vec<RidgePoint>,
    variances: Option<Vec<f64>>,
}

impl Ridge {
    /// Level that produced this ridge.
    pub fn method(&self) -> RidgeMethod {
        self.method
    }

    /// One point per frame, in increasing time.
    pub fn points(&self) -> &[RidgePoint] {
        &self.points
    }

    /// Number of frames covered.
    pub fn frame_count(&self) -> usize {
        self.points.len()
    }

    /// Per-frame position variance in bins squared, present only for a
    /// smoothed ridge.
    pub fn variances(&self) -> Option<&[f64]> {
        self.variances.as_deref()
    }
}

/// One allowed bin range per frame, inclusive at both ends.
#[derive(Debug, Clone)]
pub struct Corridor {
    bounds: Vec<(usize, usize)>,
}

impl Corridor {
    /// Build a corridor from explicit inclusive bounds.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::InvalidCorridor`] for a reversed range or one past
    /// the last bin.
    pub fn new(bounds: Vec<(usize, usize)>, bin_count: usize) -> Result<Self, DspError> {
        if bin_count == 0 {
            return Err(DspError::ZeroLength);
        }
        for &(low, high) in &bounds {
            if low > high || high >= bin_count {
                return Err(DspError::InvalidCorridor {
                    low,
                    high,
                    bins: bin_count,
                });
            }
        }
        Ok(Self { bounds })
    }

    /// Build one corridor for every frame, between two frequencies.
    ///
    /// The edges are rounded to the nearest bin and clamped to the map. The
    /// detector normally supplies a bending corridor; this constructor is for
    /// surveying and tests.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the frame count is zero, a bound is not
    /// finite, or the two frequencies cross.
    pub fn uniform(
        frame_count: usize,
        low_hz: f64,
        high_hz: f64,
        map: &ZMap,
    ) -> Result<Self, DspError> {
        if frame_count == 0 {
            return Err(DspError::ZeroLength);
        }
        if !low_hz.is_finite() || !high_hz.is_finite() || low_hz > high_hz {
            return Err(DspError::InvalidFrequencyRange { low_hz, high_hz });
        }
        let bins = map.bin_count();
        let width = map.config().bin_width_hz();
        let last = bins - 1;
        let to_bin = |hz: f64| {
            let bin = (hz / width).round();
            if bin <= 0.0 {
                0
            } else if bin >= last as f64 {
                last
            } else {
                bin as usize
            }
        };
        let low = to_bin(low_hz);
        let high = to_bin(high_hz);
        Ok(Self {
            bounds: vec![(low, high); frame_count],
        })
    }

    /// Number of frames the corridor covers.
    pub fn frame_count(&self) -> usize {
        self.bounds.len()
    }

    /// Inclusive bin range of one frame.
    pub fn bounds(&self, frame: usize) -> Option<(usize, usize)> {
        self.bounds.get(frame).copied()
    }
}

/// Settings for ridge tracking.
///
/// Every value is `PROVISIONAL(G1)`: the simulator suite fixes them once the
/// detector produces real corridors and the fitter can score a ridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RidgeConfig {
    /// Largest allowed bin change between neighbouring frames.
    pub max_step_bins: usize,
    /// Cost in z units of a step of [`Self::max_step_bins`].
    pub smoothness_cost: f64,
    /// Lowest z at which a per-frame peak counts as usable.
    pub min_peak_z: f64,
    /// Measurement variance of one bin, in bins squared.
    pub measurement_variance_bins2: f64,
    /// Process variance of the bin velocity per frame, in bins squared.
    pub process_variance_bins2: f64,
}

impl Default for RidgeConfig {
    fn default() -> Self {
        Self {
            max_step_bins: 12,
            smoothness_cost: 2.0,
            min_peak_z: 4.0,
            measurement_variance_bins2: 0.25,
            process_variance_bins2: 0.05,
        }
    }
}

impl RidgeConfig {
    /// Validate the settings.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::ZeroLength`] for a zero step, and
    /// [`DspError::InvalidVariance`] for a negative or non-finite cost or
    /// variance.
    pub fn validate(&self) -> Result<(), DspError> {
        if self.max_step_bins == 0 {
            return Err(DspError::ZeroLength);
        }
        for value in [
            self.smoothness_cost,
            self.min_peak_z,
            self.measurement_variance_bins2,
            self.process_variance_bins2,
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(DspError::InvalidVariance(value));
            }
        }
        Ok(())
    }
}

/// Track a ridge with the lowest level that works.
///
/// Runs the per-frame peaks first; if any frame's peak is below
/// [`RidgeConfig::min_peak_z`] or the path jumps more than
/// [`RidgeConfig::max_step_bins`], falls back to Viterbi; if the Viterbi path
/// still has weak frames, smooths it and reports per-frame variances.
///
/// # Errors
///
/// Returns [`DspError`] when the configuration is invalid, the map is empty, or
/// the corridor does not cover exactly the map's frames.
pub fn track_ridge(
    map: &ZMap,
    corridor: &Corridor,
    config: &RidgeConfig,
) -> Result<Ridge, DspError> {
    config.validate()?;
    check_corridor(map, corridor)?;
    let peaks = peaks(map, corridor)?;
    if is_clean(&peaks, config) {
        return Ok(peaks);
    }
    let path = viterbi(map, corridor, config)?;
    if is_clean(&path, config) {
        return Ok(path);
    }
    smoothed(map, corridor, config)
}

/// Level 1: the per-frame peak inside the corridor.
///
/// # Errors
///
/// Returns [`DspError`] when the corridor does not match the map.
pub fn peaks(map: &ZMap, corridor: &Corridor) -> Result<Ridge, DspError> {
    check_corridor(map, corridor)?;
    let mut bins = Vec::with_capacity(map.frame_count());
    for frame in 0..map.frame_count() {
        let (low, high) = corridor.bounds(frame).ok_or(DspError::CorridorMismatch {
            expected: map.frame_count(),
            actual: corridor.frame_count(),
        })?;
        let mut best_bin = low;
        let mut best_value = f64::NEG_INFINITY;
        for bin in low..=high {
            let value = f64::from(map.value(frame, bin).unwrap_or(0.0));
            if value > best_value {
                best_value = value;
                best_bin = bin;
            }
        }
        bins.push(best_bin);
    }
    Ok(Ridge::from_bins(map, &bins, RidgeMethod::Peaks))
}

/// Level 2: Viterbi path with a smoothness cost on the bin change.
///
/// The path maximizes the sum of z values minus
/// `smoothness_cost * (step / max_step_bins)^2`, and never steps further than
/// [`RidgeConfig::max_step_bins`]. A frame whose corridor is unreachable from
/// the previous one restarts the path there.
///
/// # Errors
///
/// Returns [`DspError`] when the configuration or corridor is invalid.
pub fn viterbi(map: &ZMap, corridor: &Corridor, config: &RidgeConfig) -> Result<Ridge, DspError> {
    config.validate()?;
    check_corridor(map, corridor)?;
    let frames = map.frame_count();
    if frames == 0 {
        return Err(DspError::EmptyInput);
    }
    let mut scores: Vec<Vec<f64>> = Vec::with_capacity(frames);
    let mut parents: Vec<Vec<Option<usize>>> = Vec::with_capacity(frames);

    for frame in 0..frames {
        let (low, high) = bounds_at(corridor, frame, frames)?;
        let width = high.saturating_sub(low) + 1;
        let mut frame_scores = vec![f64::NEG_INFINITY; width];
        let mut frame_parents = vec![None; width];
        if frame == 0 {
            for offset in 0..width {
                if let (Some(slot), Some(value)) =
                    (frame_scores.get_mut(offset), map.value(frame, low + offset))
                {
                    *slot = f64::from(value);
                }
            }
            scores.push(frame_scores);
            parents.push(frame_parents);
            continue;
        }

        let (previous_low, previous_high) = bounds_at(corridor, frame - 1, frames)?;
        for offset in 0..width {
            let bin = low + offset;
            let emission = f64::from(map.value(frame, bin).unwrap_or(0.0));
            let Some(previous_scores) = scores.get(frame - 1) else {
                continue;
            };
            let mut best = f64::NEG_INFINITY;
            let mut best_parent = None;
            let start = bin.saturating_sub(config.max_step_bins).max(previous_low);
            let end = bin.saturating_add(config.max_step_bins).min(previous_high);
            for previous_bin in start..=end {
                let previous_offset = previous_bin.saturating_sub(previous_low);
                let Some(&previous_score) = previous_scores.get(previous_offset) else {
                    continue;
                };
                let step = bin as f64 - previous_bin as f64;
                let normalized = step / config.max_step_bins as f64;
                let penalty = config.smoothness_cost * normalized * normalized;
                let candidate = previous_score + emission - penalty;
                if candidate > best {
                    best = candidate;
                    best_parent = Some(previous_offset);
                }
            }
            if let (Some(slot), Some(parent_slot)) =
                (frame_scores.get_mut(offset), frame_parents.get_mut(offset))
            {
                if best_parent.is_some() {
                    *slot = best;
                    *parent_slot = best_parent;
                } else {
                    // No reachable predecessor: the corridor restarted here.
                    *slot = emission;
                }
            }
        }
        scores.push(frame_scores);
        parents.push(frame_parents);
    }

    let last = frames - 1;
    let Some(last_scores) = scores.get(last) else {
        return Err(DspError::EmptyInput);
    };
    let Some(mut offset) = argmax_index(last_scores) else {
        return Err(DspError::EmptyInput);
    };
    let mut bins = vec![0_usize; frames];
    for frame in (0..frames).rev() {
        let (low, _high) = bounds_at(corridor, frame, frames)?;
        if let Some(slot) = bins.get_mut(frame) {
            *slot = low.saturating_add(offset);
        }
        if frame == 0 {
            break;
        }
        offset = match parents.get(frame).and_then(|row| row.get(offset)).copied() {
            Some(Some(parent)) => parent,
            _ => 0,
        };
    }
    Ok(Ridge::from_bins(map, &bins, RidgeMethod::Viterbi))
}

/// Level 3: Kalman filter with an RTS smoother over the Viterbi path.
///
/// The state is `(bin, bin per frame)` with a constant-velocity model. Process
/// and measurement variances come from [`RidgeConfig`]. The smoothed position
/// variance is reported per frame in bins squared.
///
/// # Errors
///
/// Returns [`DspError`] when the configuration or corridor is invalid.
pub fn smoothed(map: &ZMap, corridor: &Corridor, config: &RidgeConfig) -> Result<Ridge, DspError> {
    let path = viterbi(map, corridor, config)?;
    let measurements: Vec<f64> = path.points().iter().map(|point| point.bin).collect();
    if measurements.is_empty() {
        return Err(DspError::EmptyInput);
    }
    let (predicted, filtered) = forward_pass(&measurements, config)?;
    let (positions, variances) = backward_pass(&predicted, &filtered);
    Ok(Ridge::from_smoothed(map, &positions, &variances))
}

/// Kalman forward pass: prediction and measurement update per frame.
fn forward_pass(
    measurements: &[f64],
    config: &RidgeConfig,
) -> Result<(Vec<PriorEstimate>, Vec<FilteredEstimate>), DspError> {
    let measurement_variance = config.measurement_variance_bins2;
    let process_variance = config.process_variance_bins2;
    let mut predicted = Vec::with_capacity(measurements.len());
    let mut filtered = Vec::with_capacity(measurements.len());
    let mut position = measurements.first().copied().unwrap_or(0.0);
    let mut velocity = 0.0;
    let mut covariance = [measurement_variance, 0.0, measurement_variance];

    for &measurement in measurements {
        let prior_position = position + velocity;
        let prior_velocity = velocity;
        let prior00 = covariance[0] + 2.0 * covariance[1] + covariance[2] + process_variance / 4.0;
        let prior01 = covariance[1] + covariance[2] + process_variance / 2.0;
        let prior11 = covariance[2] + process_variance;
        predicted.push((prior_position, prior_velocity, prior00, prior01, prior11));

        let innovation = measurement - prior_position;
        let innovation_variance = prior00 + measurement_variance;
        if !innovation_variance.is_finite() || innovation_variance <= 0.0 {
            return Err(DspError::InvalidVariance(innovation_variance));
        }
        let gain_position = prior00 / innovation_variance;
        let gain_velocity = prior01 / innovation_variance;
        position = prior_position + gain_position * innovation;
        velocity = prior_velocity + gain_velocity * innovation;
        covariance = [
            prior00 * (1.0 - gain_position),
            prior01 * (1.0 - gain_position),
            prior11 - gain_velocity * prior01,
        ];
        filtered.push((position, velocity, covariance));
    }
    Ok((predicted, filtered))
}

/// Rauch-Tung-Striebel backward pass, returning smoothed positions and position
/// variances.
fn backward_pass(
    predicted: &[PriorEstimate],
    filtered: &[FilteredEstimate],
) -> (Vec<f64>, Vec<f64>) {
    let frames = filtered.len();
    let mut state = vec![[0.0_f64; 2]; frames];
    let mut covariance = vec![[0.0_f64; 3]; frames];
    let Some(last) = frames.checked_sub(1) else {
        return (Vec::new(), Vec::new());
    };
    if let (
        Some(&(position, velocity, filtered_covariance)),
        Some(state_slot),
        Some(covariance_slot),
    ) = (
        filtered.get(last),
        state.get_mut(last),
        covariance.get_mut(last),
    ) {
        *state_slot = [position, velocity];
        *covariance_slot = filtered_covariance;
    }

    for frame in (0..last).rev() {
        let (
            Some(&(filtered_position, filtered_velocity, filter_covariance)),
            Some(&(prior_position, prior_velocity, prior00, prior01, prior11)),
        ) = (filtered.get(frame), predicted.get(frame + 1))
        else {
            continue;
        };
        let (Some(&next_state), Some(&next_covariance)) =
            (state.get(frame + 1), covariance.get(frame + 1))
        else {
            continue;
        };

        // C = P_f F^T (P_prior)^-1, with F the constant-velocity transition and
        // both covariances symmetric.
        let a00 = filter_covariance[0] + filter_covariance[1];
        let a01 = filter_covariance[1];
        let a10 = filter_covariance[1] + filter_covariance[2];
        let a11 = filter_covariance[2];
        let determinant = prior00 * prior11 - prior01 * prior01;
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            // A singular prior leaves the filtered estimate in place.
            if let (Some(state_slot), Some(covariance_slot)) =
                (state.get_mut(frame), covariance.get_mut(frame))
            {
                *state_slot = [filtered_position, filtered_velocity];
                *covariance_slot = filter_covariance;
            }
            continue;
        }
        let c00 = (a00 * prior11 - a01 * prior01) / determinant;
        let c01 = (a01 * prior00 - a00 * prior01) / determinant;
        let c10 = (a10 * prior11 - a11 * prior01) / determinant;
        let c11 = (a11 * prior00 - a10 * prior01) / determinant;

        let delta_position = next_state[0] - prior_position;
        let delta_velocity = next_state[1] - prior_velocity;
        let smoothed_position = filtered_position + c00 * delta_position + c01 * delta_velocity;
        let smoothed_velocity = filtered_velocity + c10 * delta_position + c11 * delta_velocity;

        // P_s = P_f + C (P_s[next] - P_prior[next]) C^T.
        let d00 = next_covariance[0] - prior00;
        let d01 = next_covariance[1] - prior01;
        let d11 = next_covariance[2] - prior11;
        let e00 = c00 * d00 + c01 * d01;
        let e01 = c00 * d01 + c01 * d11;
        let e10 = c10 * d00 + c11 * d01;
        let e11 = c10 * d01 + c11 * d11;
        let smoothed00 = filter_covariance[0] + e00 * c00 + e01 * c01;
        let smoothed01 = filter_covariance[1] + e00 * c10 + e01 * c11;
        let smoothed11 = filter_covariance[2] + e10 * c10 + e11 * c11;

        if let (Some(state_slot), Some(covariance_slot)) =
            (state.get_mut(frame), covariance.get_mut(frame))
        {
            *state_slot = [smoothed_position, smoothed_velocity];
            *covariance_slot = [smoothed00.max(0.0), smoothed01, smoothed11.max(0.0)];
        }
    }

    let positions = state.iter().map(|entry| entry[0]).collect();
    let variances = covariance.iter().map(|entry| entry[0].max(0.0)).collect();
    (positions, variances)
}

/// Inclusive bin bounds of one frame, or a corridor mismatch.
fn bounds_at(corridor: &Corridor, frame: usize, frames: usize) -> Result<(usize, usize), DspError> {
    corridor.bounds(frame).ok_or(DspError::CorridorMismatch {
        expected: frames,
        actual: corridor.frame_count(),
    })
}

/// Whether a ridge is clean enough to keep at its level.
fn is_clean(ridge: &Ridge, config: &RidgeConfig) -> bool {
    let points = ridge.points();
    if points.iter().any(|point| point.z < config.min_peak_z) {
        return false;
    }
    points
        .windows(2)
        .all(|pair| match (pair.first(), pair.get(1)) {
            (Some(previous), Some(next)) => {
                (next.bin - previous.bin).abs() <= config.max_step_bins as f64
            }
            _ => true,
        })
}

/// Check that the corridor covers exactly the map's frames.
fn check_corridor(map: &ZMap, corridor: &Corridor) -> Result<(), DspError> {
    if corridor.frame_count() != map.frame_count() {
        return Err(DspError::CorridorMismatch {
            expected: map.frame_count(),
            actual: corridor.frame_count(),
        });
    }
    if map.frame_count() == 0 {
        return Err(DspError::EmptyInput);
    }
    Ok(())
}

/// Index of the first maximum, or `None` for an empty slice.
fn argmax_index(values: &[f64]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, &value) in values.iter().enumerate() {
        match best {
            Some((_, best_value)) if value <= best_value => {}
            _ => best = Some((index, value)),
        }
    }
    best.map(|(index, _value)| index)
}

impl Ridge {
    /// Build a ridge from integer bin positions.
    fn from_bins(map: &ZMap, bins: &[usize], method: RidgeMethod) -> Self {
        let width = map.config().bin_width_hz();
        let points = bins
            .iter()
            .enumerate()
            .map(|(frame, &bin)| RidgePoint {
                bin: bin as f64,
                frequency_hz: bin as f64 * width,
                z: f64::from(map.value(frame, bin).unwrap_or(0.0)),
            })
            .collect();
        Self {
            method,
            points,
            variances: None,
        }
    }

    /// Build a ridge from fractional bin positions and per-frame variances.
    fn from_smoothed(map: &ZMap, bins: &[f64], variances: &[f64]) -> Self {
        let width = map.config().bin_width_hz();
        let points = bins
            .iter()
            .enumerate()
            .map(|(frame, &bin)| {
                let rounded = if bin <= 0.0 { 0 } else { bin.round() as usize };
                RidgePoint {
                    bin,
                    frequency_hz: bin * width,
                    z: f64::from(map.value(frame, rounded).unwrap_or(0.0)),
                }
            })
            .collect();
        Self {
            method: RidgeMethod::Smoothed,
            points,
            variances: Some(variances.to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Corridor, RidgeConfig, RidgeMethod, peaks, track_ridge};
    use crate::stft::StftConfig;
    use crate::whiten::ZMap;

    const RATE_HZ: f64 = 4_000.0;

    fn survey_config() -> StftConfig {
        StftConfig::survey(RATE_HZ).expect("survey configuration")
    }

    /// A map whose ridge sits at `ridge_bin(frame)`, with the given height.
    fn synthetic_map(frames: usize, ridge_bin: impl Fn(usize) -> usize, height: f32) -> ZMap {
        let config = survey_config();
        let bins = config.bin_count();
        let mut values = vec![0.5_f32; frames * bins];
        for frame in 0..frames {
            let bin = ridge_bin(frame);
            if let Some(slot) = values.get_mut(frame * bins + bin) {
                *slot = height;
            }
        }
        let frame_times: Vec<f64> = (0..frames).map(|frame| frame as f64 * 0.25).collect();
        ZMap::from_parts(config, frame_times, values).expect("synthetic map")
    }

    /// A map with a ridge plus one brighter outlier cell.
    fn synthetic_map_with_outlier(
        frames: usize,
        ridge_bin: impl Fn(usize) -> usize,
        ridge_height: f32,
        outlier: (usize, usize, f32),
    ) -> ZMap {
        let config = survey_config();
        let bins = config.bin_count();
        let mut values = vec![0.5_f32; frames * bins];
        for frame in 0..frames {
            if let Some(slot) = values.get_mut(frame * bins + ridge_bin(frame)) {
                *slot = ridge_height;
            }
        }
        if let Some(slot) = values.get_mut(outlier.0 * bins + outlier.1) {
            *slot = outlier.2;
        }
        let frame_times: Vec<f64> = (0..frames).map(|frame| frame as f64 * 0.25).collect();
        ZMap::from_parts(config, frame_times, values).expect("synthetic map")
    }

    fn corridor_for(map: &ZMap, low_bin: usize, high_bin: usize) -> Corridor {
        Corridor::new(
            vec![(low_bin, high_bin); map.frame_count()],
            map.bin_count(),
        )
        .expect("corridor")
    }

    #[test]
    fn fr_dm_01_level_one_follows_a_clean_ridge() {
        let map = synthetic_map(30, |frame| 100 + 2 * frame, 20.0);
        let corridor = corridor_for(&map, 80, 180);
        let ridge = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("ridge");
        assert_eq!(ridge.method(), RidgeMethod::Peaks);
        assert_eq!(ridge.frame_count(), 30);
        for (frame, point) in ridge.points().iter().enumerate() {
            assert!(
                (point.bin - (100 + 2 * frame) as f64).abs() < 0.5,
                "frame {frame} bin {}",
                point.bin
            );
        }
        assert!(ridge.variances().is_none());
    }

    #[test]
    fn fr_dm_01_viterbi_rejects_a_single_frame_outlier() {
        // The ridge stays intact, but one frame carries a brighter cell far
        // enough away that level 1 jumps to it and breaks the step limit.
        let map = synthetic_map_with_outlier(30, |frame| 100 + 2 * frame, 20.0, (15, 160, 40.0));
        let corridor = corridor_for(&map, 80, 180);
        let peaks = peaks(&map, &corridor).expect("peaks");
        assert_eq!(peaks.points()[15].bin, 160.0, "level 1 takes the outlier");

        let ridge = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("ridge");
        assert_eq!(ridge.method(), RidgeMethod::Viterbi);
        let tracked = ridge.points()[15].bin;
        assert!(
            (tracked - 130.0).abs() <= 12.0,
            "viterbi tracked {tracked} at the outlier frame"
        );
        assert!(ridge.variances().is_none());
    }

    #[test]
    fn fr_dm_01_a_faint_ridge_is_smoothed_with_variances() {
        let map = synthetic_map(30, |frame| 100 + 3 * frame, 2.0);
        let corridor = corridor_for(&map, 80, 200);
        let ridge = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("ridge");
        assert_eq!(ridge.method(), RidgeMethod::Smoothed);
        let variances = ridge.variances().expect("variances");
        assert_eq!(variances.len(), ridge.frame_count());
        for variance in variances {
            assert!(
                variance.is_finite() && *variance > 0.0,
                "variance {variance}"
            );
        }
        for (frame, point) in ridge.points().iter().enumerate() {
            assert!(
                (point.bin - (100 + 3 * frame) as f64).abs() < 1.5,
                "frame {frame} smoothed bin {}",
                point.bin
            );
        }
    }

    #[test]
    fn uniform_corridor_converts_frequencies_to_bins() {
        let map = synthetic_map(4, |_| 200, 10.0);
        let corridor = Corridor::uniform(4, 100.0, 200.0, &map).expect("corridor");
        let (low, high) = corridor.bounds(0).expect("bounds");
        // Bin width is 4000 / 8192 Hz, so 100 Hz is bin 205 and 200 Hz bin 410.
        assert_eq!(low, 205);
        assert_eq!(high, 410);
        assert_eq!(corridor.frame_count(), 4);
        assert!(corridor.bounds(4).is_none());
    }

    #[test]
    fn corridor_length_must_match_the_map() {
        let map = synthetic_map(10, |_| 200, 10.0);
        let corridor = corridor_for(&map, 100, 300);
        assert_eq!(corridor.frame_count(), 10);
        let short = Corridor::new(vec![(100, 300); 4], map.bin_count()).expect("corridor");
        assert!(track_ridge(&map, &short, &RidgeConfig::default()).is_err());
    }

    #[test]
    fn reversed_corridor_bounds_are_rejected() {
        let map = synthetic_map(4, |_| 200, 10.0);
        let result = Corridor::new(vec![(300, 100)], map.bin_count());
        assert!(result.is_err());
        let past_end = Corridor::new(vec![(100, map.bin_count())], map.bin_count());
        assert!(past_end.is_err());
    }

    #[test]
    fn empty_corridor_is_rejected() {
        let map = synthetic_map(4, |_| 200, 10.0);
        let empty = Corridor::new(Vec::new(), map.bin_count()).expect("corridor");
        assert!(track_ridge(&map, &empty, &RidgeConfig::default()).is_err());
    }

    #[test]
    fn invalid_configuration_is_rejected() {
        let map = synthetic_map(4, |_| 200, 10.0);
        let corridor = corridor_for(&map, 100, 300);
        let config = RidgeConfig {
            max_step_bins: 0,
            ..RidgeConfig::default()
        };
        assert!(track_ridge(&map, &corridor, &config).is_err());
        let config = RidgeConfig {
            measurement_variance_bins2: -1.0,
            ..RidgeConfig::default()
        };
        assert!(track_ridge(&map, &corridor, &config).is_err());
    }

    #[test]
    fn tracking_is_deterministic() {
        let map = synthetic_map_with_outlier(30, |frame| 100 + 2 * frame, 20.0, (15, 160, 40.0));
        let corridor = corridor_for(&map, 80, 180);
        let first = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("first");
        let second = track_ridge(&map, &corridor, &RidgeConfig::default()).expect("second");
        assert_eq!(first.method(), second.method());
        for (left, right) in first.points().iter().zip(second.points().iter()) {
            assert_eq!(left.bin, right.bin);
        }
    }
}
