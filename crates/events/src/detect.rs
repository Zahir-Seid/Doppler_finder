//! Generalized likelihood-ratio detector for a propeller pass (FR-DM-01).
//!
//! `docs/SDD.md` section 5.3 fixes the method: Doppler templates over a grid of
//! speed, closest distance, fundamental and closest-approach time, correlated
//! against the whitened z-map. Both the approach half and the recede half must
//! each clear their own threshold, which rejects a steady hum or a single
//! click. Thresholds follow a Neyman-Pearson rule at a configured false-alarm
//! rate per hour, with the null measured on time-reversed and frequency-shifted
//! copies of the same map, so nothing here assumes a particular noise model.
//!
//! The detector only proposes hypotheses. It does not fit, weight or judge
//! them; that is FR-DM-02 and FR-DM-06.

use core_types::{AudioSeconds, Hertz};
use dsp::ZMap;

use crate::error::EventError;

/// Largest number of values a grid may generate, protecting the search cost.
const MAX_GRID_VALUES: usize = 512;

/// An inclusive, evenly spaced grid of parameter values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridRange {
    /// First value.
    pub min: f64,
    /// Last value that `min + k * step` does not pass.
    pub max: f64,
    /// Spacing between values.
    pub step: f64,
}

impl GridRange {
    /// Build a grid.
    pub fn new(min: f64, max: f64, step: f64) -> Self {
        Self { min, max, step }
    }

    /// Number of values the grid generates.
    pub fn count(&self) -> usize {
        let span = (self.max - self.min) / self.step;
        if !span.is_finite() || span < 0.0 {
            return 0;
        }
        (span.floor() as usize).saturating_add(1)
    }

    /// The grid's values, from `min` upwards in steps.
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        (0..self.count()).map(move |index| self.min + index as f64 * self.step)
    }

    /// Check that the grid is usable and not larger than the search budget.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite bound,
    /// [`EventError::InvalidGrid`] for a non-positive step or a reversed range,
    /// and [`EventError::GridTooLarge`] when the grid exceeds the budget.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.min.is_finite() || !self.max.is_finite() || !self.step.is_finite() {
            return Err(EventError::NotFinite);
        }
        if self.step <= 0.0 || self.max < self.min {
            return Err(EventError::InvalidGrid {
                min: self.min,
                max: self.max,
                step: self.step,
            });
        }
        let count = self.count();
        if count == 0 || count > MAX_GRID_VALUES {
            return Err(EventError::GridTooLarge(count));
        }
        Ok(())
    }
}

/// One candidate pass: a straight-line constant-speed approach.
///
/// The geometry is the one SDD section 4 uses for the fit: the aircraft passes
/// `closest_m` from the observer at `speed_m_per_s`, with the closest approach
/// heard at `heard_at` on the audio clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventHypothesis {
    /// Heard closest-approach time on the audio clock.
    pub heard_at: AudioSeconds,
    /// Fundamental frequency of the propeller, in hertz.
    pub fundamental_hz: Hertz,
    /// Speed over the ground, in metres per second.
    pub speed_m_per_s: f64,
    /// Closest approach distance, in metres.
    pub closest_m: f64,
}

impl EventHypothesis {
    /// Radial velocity toward the observer at `t_rel_s` seconds from closest
    /// approach, in metres per second. Positive while the aircraft approaches.
    pub fn radial_velocity_m_per_s(&self, t_rel_s: f64) -> f64 {
        let along_m = self.speed_m_per_s * t_rel_s;
        let range_m = (self.closest_m * self.closest_m + along_m * along_m).sqrt();
        if !range_m.is_finite() || range_m <= 0.0 {
            return 0.0;
        }
        -(self.speed_m_per_s * along_m) / range_m
    }

    /// Phase Doppler frequency received at `t_rel_s` seconds from closest
    /// approach, in hertz (`f0 c / (c - v_r)`).
    ///
    /// Returns `None` where the model has no finite answer, which the detector
    /// treats as an unusable hypothesis.
    pub fn observed_hz(&self, t_rel_s: f64, sound_speed_m_per_s: f64) -> Option<f64> {
        let radial_m_per_s = self.radial_velocity_m_per_s(t_rel_s);
        let denominator = sound_speed_m_per_s - radial_m_per_s;
        if !denominator.is_finite() || denominator <= 0.0 {
            return None;
        }
        let frequency_hz = self.fundamental_hz.0 * sound_speed_m_per_s / denominator;
        if frequency_hz.is_finite() {
            Some(frequency_hz)
        } else {
            None
        }
    }
}

/// Settings for the detector.
///
/// The grids are searched exhaustively, so their product sets the cost. The
/// default grid is `PROVISIONAL(G1)`: the simulator suite replaces it once the
/// event fitter can score a detection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Candidate speeds over the ground, in metres per second.
    pub speeds_m_per_s: GridRange,
    /// Candidate closest-approach distances, in metres.
    pub closest_m: GridRange,
    /// Candidate fundamental frequencies, in hertz.
    pub fundamental_hz: GridRange,
    /// Speed of sound used by the template, in metres per second.
    pub sound_speed_m_per_s: f64,
    /// Accepted false alarms per hour of map, setting the threshold.
    pub false_alarm_rate_per_hour: f64,
    /// Frames each of the approach and recede halves must contain.
    pub min_frames_per_half: usize,
    /// Frequency shift of the null maps, as a fraction of the bin count.
    pub null_shift_fraction: f64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            speeds_m_per_s: GridRange::new(40.0, 180.0, 20.0),
            closest_m: GridRange::new(200.0, 1_200.0, 200.0),
            fundamental_hz: GridRange::new(60.0, 420.0, 20.0),
            sound_speed_m_per_s: 343.0,
            false_alarm_rate_per_hour: 1.0,
            min_frames_per_half: 3,
            null_shift_fraction: 0.125,
        }
    }
}

impl DetectorConfig {
    /// Check the settings before a search.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite sound speed,
    /// [`EventError::InvalidGrid`] for a bad grid,
    /// [`EventError::GridTooLarge`] for an over-budget grid, and
    /// [`EventError::InvalidRate`] for a false-alarm rate or shift outside its
    /// usable range.
    pub fn validate(&self) -> Result<(), EventError> {
        self.speeds_m_per_s.validate()?;
        self.closest_m.validate()?;
        self.fundamental_hz.validate()?;
        if !self.sound_speed_m_per_s.is_finite() || self.sound_speed_m_per_s <= 0.0 {
            return Err(EventError::NotFinite);
        }
        if !self.false_alarm_rate_per_hour.is_finite() || self.false_alarm_rate_per_hour <= 0.0 {
            return Err(EventError::InvalidRate(self.false_alarm_rate_per_hour));
        }
        if !self.null_shift_fraction.is_finite() || !(0.0..1.0).contains(&self.null_shift_fraction)
        {
            return Err(EventError::InvalidRate(self.null_shift_fraction));
        }
        if self.min_frames_per_half == 0 {
            return Err(EventError::ZeroLength);
        }
        Ok(())
    }

    /// Number of hypotheses one map is searched with, for cost budgeting.
    pub fn hypothesis_count(&self, time_candidates: usize) -> usize {
        self.speeds_m_per_s
            .count()
            .saturating_mul(self.closest_m.count())
            .saturating_mul(self.fundamental_hz.count())
            .saturating_mul(time_candidates)
    }
}

/// The best hypothesis the detector found, with the statistics behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    /// Best-scoring hypothesis.
    pub hypothesis: EventHypothesis,
    /// Mean z along the whole template curve.
    pub score: f64,
    /// Mean z along the approach half.
    pub approach_score: f64,
    /// Mean z along the recede half.
    pub recede_score: f64,
    /// Threshold the whole-curve score must reach.
    pub threshold: f64,
    /// Threshold the approach half must reach.
    pub approach_threshold: f64,
    /// Threshold the recede half must reach.
    pub recede_threshold: f64,
    /// Number of null maps the thresholds were measured on.
    pub null_maps: usize,
    /// Whether every statistic cleared its threshold.
    pub accepted: bool,
}

/// How a null map is derived from the real one.
#[derive(Debug, Clone, Copy)]
enum MapView {
    /// The map as measured.
    Direct,
    /// Frames in reverse order.
    Mirrored,
    /// Bins rotated by a fixed offset.
    Shifted(usize),
}

impl MapView {
    /// Value at one frame and bin under this view.
    fn value(self, map: &ZMap, frame: usize, bin: usize) -> Option<f32> {
        let bin = match self {
            MapView::Shifted(offset) => {
                let bins = map.bin_count();
                if bins == 0 {
                    return None;
                }
                bin.checked_add(offset)?.checked_rem(bins)?
            }
            MapView::Direct | MapView::Mirrored => bin,
        };
        let frame = match self {
            MapView::Mirrored => map.frame_count().checked_sub(1)?.checked_sub(frame)?,
            MapView::Direct | MapView::Shifted(_) => frame,
        };
        map.value(frame, bin)
    }
}

/// Scores of one hypothesis under one view.
#[derive(Debug, Clone, Copy)]
struct Scores {
    hypothesis: EventHypothesis,
    total: f64,
    approach: f64,
    recede: f64,
}

/// Run the detector over a whitened map and report the best hypothesis.
///
/// The search maximizes the mean z along each template curve. A hypothesis is
/// eligible only when both halves hold at least
/// [`DetectorConfig::min_frames_per_half`] frames, so a curve cannot win by
/// covering one half alone. Thresholds come from the same search run over
/// time-reversed and frequency-shifted copies of the map.
///
/// # Errors
///
/// Returns [`EventError`] when the configuration is invalid, the map is empty,
/// or no hypothesis produces a usable curve at all.
pub fn detect(map: &ZMap, config: &DetectorConfig) -> Result<Detection, EventError> {
    config.validate()?;
    let frame_count = map.frame_count();
    let bins = map.bin_count();
    if frame_count == 0 || bins == 0 {
        return Err(EventError::EmptyMap);
    }
    let times = map.frame_times_s();
    if times.iter().any(|time| !time.is_finite()) {
        return Err(EventError::NotFinite);
    }
    if frame_count < config.min_frames_per_half.saturating_mul(2) {
        return Err(EventError::InsufficientFrames {
            needed: config.min_frames_per_half.saturating_mul(2),
            actual: frame_count,
        });
    }

    let best = search(map, MapView::Direct, config).ok_or(EventError::NoHypothesis)?;
    let views = null_views(map, config);
    let null_maps = views.len();
    let mut totals = Vec::with_capacity(null_maps);
    let mut approaches = Vec::with_capacity(null_maps);
    let mut recedes = Vec::with_capacity(null_maps);
    for view in views {
        if let Some(scores) = search(map, view, config) {
            totals.push(scores.total);
            approaches.push(scores.approach);
            recedes.push(scores.recede);
        }
    }
    if totals.is_empty() {
        return Err(EventError::NoHypothesis);
    }

    // The false-alarm rate is per map; with a handful of null maps the
    // empirical quantile is the maximum, which is the conservative choice.
    let first = times.first().copied().unwrap_or(0.0);
    let last = times.last().copied().unwrap_or(0.0);
    let hours = (last - first).abs() / 3_600.0;
    let alpha = (config.false_alarm_rate_per_hour * hours).clamp(1.0e-9, 0.5);
    let threshold = quantile(&mut totals, alpha).ok_or(EventError::NoHypothesis)?;
    let approach_threshold = quantile(&mut approaches, alpha).ok_or(EventError::NoHypothesis)?;
    let recede_threshold = quantile(&mut recedes, alpha).ok_or(EventError::NoHypothesis)?;

    Ok(Detection {
        hypothesis: best.hypothesis,
        score: best.total,
        approach_score: best.approach,
        recede_score: best.recede,
        threshold,
        approach_threshold,
        recede_threshold,
        null_maps,
        accepted: best.total >= threshold
            && best.approach >= approach_threshold
            && best.recede >= recede_threshold,
    })
}

/// The best-scoring hypothesis under one view.
fn search(map: &ZMap, view: MapView, config: &DetectorConfig) -> Option<Scores> {
    let bins = map.bin_count();
    let width_hz = map.config().bin_width_hz();
    let mut best: Option<Scores> = None;
    for speed_m_per_s in config.speeds_m_per_s.values() {
        for closest_m in config.closest_m.values() {
            for fundamental_hz in config.fundamental_hz.values() {
                for &heard_at in map.frame_times_s() {
                    let hypothesis = EventHypothesis {
                        heard_at: AudioSeconds(heard_at),
                        fundamental_hz: Hertz(fundamental_hz),
                        speed_m_per_s,
                        closest_m,
                    };
                    let Some(scores) =
                        score_hypothesis(map, view, &hypothesis, config, bins, width_hz)
                    else {
                        continue;
                    };
                    if best
                        .as_ref()
                        .is_none_or(|current| scores.total > current.total)
                    {
                        best = Some(scores);
                    }
                }
            }
        }
    }
    best
}

/// Score one hypothesis against one view of the map.
///
/// Returns `None` when any predicted frequency falls outside the map, which
/// makes the hypothesis unusable rather than merely weak.
fn score_hypothesis(
    map: &ZMap,
    view: MapView,
    hypothesis: &EventHypothesis,
    config: &DetectorConfig,
    bins: usize,
    width_hz: f64,
) -> Option<Scores> {
    let mut approach_sum = 0.0_f64;
    let mut approach_count = 0_usize;
    let mut recede_sum = 0.0_f64;
    let mut recede_count = 0_usize;
    for (frame, &time_s) in map.frame_times_s().iter().enumerate() {
        let t_rel_s = time_s - hypothesis.heard_at.0;
        let frequency_hz = hypothesis.observed_hz(t_rel_s, config.sound_speed_m_per_s)?;
        let bin = (frequency_hz / width_hz).round();
        if !bin.is_finite() || bin < 0.0 {
            return None;
        }
        let bin = bin as usize;
        if bin >= bins {
            return None;
        }
        let value = f64::from(view.value(map, frame, bin)?);
        if t_rel_s < 0.0 {
            approach_sum += value;
            approach_count = approach_count.saturating_add(1);
        } else {
            recede_sum += value;
            recede_count = recede_count.saturating_add(1);
        }
    }
    if approach_count < config.min_frames_per_half || recede_count < config.min_frames_per_half {
        return None;
    }
    let approach = approach_sum / approach_count as f64;
    let recede = recede_sum / recede_count as f64;
    Some(Scores {
        hypothesis: *hypothesis,
        total: f64::midpoint(approach, recede),
        approach,
        recede,
    })
}

/// Null maps: the frame-reversed map and the map rotated by fixed bin offsets.
fn null_views(map: &ZMap, config: &DetectorConfig) -> Vec<MapView> {
    let bins = map.bin_count();
    let mut views = vec![MapView::Mirrored];
    if bins == 0 {
        return views;
    }
    let shift = (bins as f64 * config.null_shift_fraction).round().max(1.0) as usize;
    for multiplier in 1_usize..=2 {
        let Some(offset) = shift.saturating_mul(multiplier).checked_rem(bins) else {
            continue;
        };
        if offset == 0 {
            continue;
        }
        views.push(MapView::Shifted(offset));
        views.push(MapView::Shifted(bins - offset));
    }
    views
}

/// Empirical `1 - alpha` quantile of a sample, reordering it in place.
fn quantile(samples: &mut [f64], alpha: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f64::total_cmp);
    let last = samples.len().saturating_sub(1);
    let rank = ((1.0 - alpha) * samples.len() as f64).floor();
    let index = if rank.is_finite() && rank >= 0.0 {
        (rank as usize).min(last)
    } else {
        last
    };
    samples.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::{DetectorConfig, EventHypothesis, GridRange, detect};
    use crate::error::EventError;
    use core_types::{AudioSeconds, Hertz};
    use dsp::{StftConfig, ZMap};

    const RATE_HZ: f64 = 4_000.0;
    const HOP_FRAMES: usize = 1_000;
    const SOUND_SPEED: f64 = 343.0;

    fn survey_config() -> StftConfig {
        StftConfig::survey(RATE_HZ).expect("survey configuration")
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

    fn frame_times(frames: usize) -> Vec<f64> {
        (0..frames)
            .map(|frame| frame as f64 * (HOP_FRAMES as f64 / RATE_HZ))
            .collect()
    }

    /// A z-map of noise with an optional ridge added on a known curve.
    fn map_with_ridge(frames: usize, curve: impl Fn(f64) -> Option<f64>, height: f32) -> ZMap {
        let config = survey_config();
        let bins = config.bin_count();
        let width_hz = config.bin_width_hz();
        let times = frame_times(frames);
        let mut state = 0x5eed_1234_u32;
        let mut values = vec![0.0_f32; frames * bins];
        for (frame, &time_s) in times.iter().enumerate() {
            for bin in 0..bins {
                if let Some(slot) = values.get_mut(frame * bins + bin) {
                    *slot = gaussian(&mut state) as f32;
                }
            }
            let Some(frequency_hz) = curve(time_s) else {
                continue;
            };
            let centre = (frequency_hz / width_hz).round();
            if !centre.is_finite() || centre < 0.0 {
                continue;
            }
            let centre = centre as usize;
            for (offset, weight) in [(-1_isize, 0.5_f32), (0, 1.0), (1, 0.5)] {
                let bin = centre.wrapping_add_signed(offset);
                if let Some(slot) = values.get_mut(frame * bins + bin) {
                    *slot += height * weight;
                }
            }
        }
        ZMap::from_parts(config, times, values).expect("synthetic map")
    }

    fn truth() -> EventHypothesis {
        EventHypothesis {
            heard_at: AudioSeconds(2.0),
            fundamental_hz: Hertz(200.0),
            speed_m_per_s: 80.0,
            closest_m: 500.0,
        }
    }

    fn config() -> DetectorConfig {
        DetectorConfig {
            speeds_m_per_s: GridRange::new(60.0, 100.0, 20.0),
            closest_m: GridRange::new(400.0, 600.0, 100.0),
            fundamental_hz: GridRange::new(180.0, 220.0, 20.0),
            ..DetectorConfig::default()
        }
    }

    #[test]
    fn fr_dm_01_detects_an_injected_doppler_curve() {
        let truth = truth();
        let map = map_with_ridge(
            21,
            |time_s| truth.observed_hz(time_s - 2.0, SOUND_SPEED),
            20.0,
        );
        let detection = detect(&map, &config()).expect("detection");
        assert!(
            detection.accepted,
            "score {} against threshold {}",
            detection.score, detection.threshold
        );
        assert!(detection.score > detection.threshold);
        assert!(detection.approach_score > detection.approach_threshold);
        assert!(detection.recede_score > detection.recede_threshold);
        let found = detection.hypothesis;
        assert!(
            (found.speed_m_per_s - truth.speed_m_per_s).abs() <= 20.0,
            "speed {}",
            found.speed_m_per_s
        );
        assert!(
            (found.closest_m - truth.closest_m).abs() <= 100.0,
            "closest {}",
            found.closest_m
        );
        assert!(
            (found.fundamental_hz.0 - truth.fundamental_hz.0).abs() <= 20.0,
            "fundamental {}",
            found.fundamental_hz.0
        );
        assert!(
            (found.heard_at.0 - truth.heard_at.0).abs() <= 0.25,
            "heard at {}",
            found.heard_at.0
        );
        assert!(
            detection.null_maps >= 3,
            "null maps {}",
            detection.null_maps
        );
    }

    #[test]
    fn fr_dm_01_noise_alone_is_not_accepted() {
        let map = map_with_ridge(21, |_| None, 0.0);
        let detection = detect(&map, &config()).expect("detection");
        assert!(
            !detection.accepted,
            "noise scored {} against threshold {}",
            detection.score, detection.threshold
        );
    }

    #[test]
    fn fr_dm_01_a_one_sided_ramp_is_rejected() {
        // The ridge exists only on the approach side of the truth time; the
        // recede half must fail even though the approach half is strong.
        let truth = truth();
        let map = map_with_ridge(
            21,
            |time_s| {
                let t_rel = time_s - 2.0;
                (t_rel < 0.0)
                    .then(|| truth.observed_hz(t_rel, SOUND_SPEED))
                    .flatten()
            },
            20.0,
        );
        let detection = detect(&map, &config()).expect("detection");
        assert!(
            !detection.accepted,
            "one-sided ramp accepted: approach {} recede {}",
            detection.approach_score, detection.recede_score
        );
    }

    #[test]
    fn fr_dm_01_detection_is_deterministic() {
        let truth = truth();
        let map = map_with_ridge(
            21,
            |time_s| truth.observed_hz(time_s - 2.0, SOUND_SPEED),
            20.0,
        );
        let config = config();
        let first = detect(&map, &config).expect("first");
        let second = detect(&map, &config).expect("second");
        assert_eq!(first.hypothesis, second.hypothesis);
        assert_eq!(first.score.to_bits(), second.score.to_bits());
        assert_eq!(first.threshold.to_bits(), second.threshold.to_bits());
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let map = map_with_ridge(21, |_| None, 0.0);
        let zero_step = DetectorConfig {
            speeds_m_per_s: GridRange::new(40.0, 180.0, 0.0),
            ..config()
        };
        assert!(matches!(
            detect(&map, &zero_step),
            Err(EventError::InvalidGrid { .. })
        ));
        let reversed = DetectorConfig {
            closest_m: GridRange::new(600.0, 400.0, 100.0),
            ..config()
        };
        assert!(matches!(
            detect(&map, &reversed),
            Err(EventError::InvalidGrid { .. })
        ));
        let rate = DetectorConfig {
            false_alarm_rate_per_hour: 0.0,
            ..config()
        };
        assert!(matches!(
            detect(&map, &rate),
            Err(EventError::InvalidRate(_))
        ));
        let shift = DetectorConfig {
            null_shift_fraction: 1.5,
            ..config()
        };
        assert!(matches!(
            detect(&map, &shift),
            Err(EventError::InvalidRate(_))
        ));
    }

    #[test]
    fn a_short_map_is_rejected() {
        let map = map_with_ridge(4, |_| None, 0.0);
        assert!(matches!(
            detect(&map, &config()),
            Err(EventError::InsufficientFrames { .. })
        ));
    }

    #[test]
    fn hypothesis_count_matches_the_grid_product() {
        let config = config();
        assert_eq!(config.speeds_m_per_s.count(), 3);
        assert_eq!(config.closest_m.count(), 3);
        assert_eq!(config.fundamental_hz.count(), 3);
        assert_eq!(config.hypothesis_count(21), 3 * 3 * 3 * 21);
    }

    #[test]
    fn grid_values_stop_at_the_last_step_inside_the_range() {
        let grid = GridRange::new(0.0, 1.0, 0.3);
        let values: Vec<f64> = grid.values().collect();
        assert_eq!(values.len(), 4);
        assert_eq!(values.first().copied(), Some(0.0));
        assert!((values.last().copied().unwrap_or(0.0) - 0.9).abs() < 1.0e-12);
    }
}
