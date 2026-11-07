//! Per-event quality score and gate (FR-DM-06).
//!
//! `docs/SDD.md` section 5.6 combines four diagnostics measured on a fitted
//! event: the signal-to-noise ratio along the model curve, the number of
//! visible harmonics, the residual autocorrelation and the covariance
//! inflation factor. Each becomes a bounded component in `[0, 1]`; their mean
//! is the event score. An event is retained only when no hard gate fires and
//! the score reaches the configured cut. Dropped events keep their reasons so
//! the run report can say why.
//!
//! The curve SNR is `10·log10(mean z²)` along the fitted curve. Whitened noise
//! has unit variance, so a noise-only curve sits near 0 dB; a coherent ridge
//! raises the mean quickly.
//!
//! The defaults are `PROVISIONAL(G1)`: the simulator suite fixes the thresholds
//! once synthetic events can be scored across SNR.

use core_types::Hertz;
use dsp::ZMap;

use crate::detect::EventHypothesis;
use crate::error::EventError;
use crate::fit::{FitReport, Parameters};
use crate::sampling;

/// A bounded quality score.
///
/// The score combines curve SNR, harmonic count, residual autocorrelation and
/// covariance inflation (SDD section 5.6). Larger is better and the range is
/// `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct EventQuality {
    /// Score in `[0, 1]`, where larger is better.
    pub score: f64,
}

impl EventQuality {
    /// Build a score, rejecting non-finite input and clamping to `[0, 1]`.
    pub fn from_score(score: f64) -> Result<Self, EventError> {
        if !score.is_finite() {
            return Err(EventError::NotFinite);
        }
        Ok(Self {
            score: score.clamp(0.0, 1.0),
        })
    }

    /// Return whether the event survives a configured cut.
    pub fn is_retained(&self, cut: f64) -> bool {
        self.score >= cut
    }
}

/// Thresholds for the quality score and gate.
///
/// Every field is a starting value from `PROVISIONAL(G1)`; the simulator suite
/// replaces them once event quality can be calibrated against truth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityConfig {
    /// Speed of sound used to sample the fitted curve, metres per second.
    pub sound_speed_m_per_s: f64,
    /// Curve SNR below which an event is dropped, decibels.
    pub min_curve_snr_db: f64,
    /// Curve SNR at or above which the SNR component scores one, decibels.
    pub full_curve_snr_db: f64,
    /// Harmonics required for the event to be retained.
    pub min_harmonics: usize,
    /// Harmonics at or above which the harmonic component scores one.
    pub target_harmonics: usize,
    /// Largest positive residual autocorrelation allowed, in `[0, 1]`.
    pub max_residual_autocorrelation: f64,
    /// Largest covariance inflation allowed; one means no inflation.
    pub max_covariance_inflation: f64,
    /// Score below which an event is dropped, in `[0, 1]`.
    pub retention_score: f64,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            sound_speed_m_per_s: 343.0,
            min_curve_snr_db: 6.0,
            full_curve_snr_db: 20.0,
            min_harmonics: 1,
            target_harmonics: 3,
            max_residual_autocorrelation: 0.5,
            max_covariance_inflation: 25.0,
            retention_score: 0.5,
        }
    }
}

impl QualityConfig {
    /// Check the settings before scoring an event.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::NotFinite`] for a non-finite sound speed or SNR
    /// bound, [`EventError::InvalidRate`] when the full-score SNR does not
    /// exceed the minimum, when a correlation or score falls outside its range,
    /// or when the inflation bound is below one, and [`EventError::ZeroLength`]
    /// when no harmonic is required.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.sound_speed_m_per_s.is_finite() || self.sound_speed_m_per_s <= 0.0 {
            return Err(EventError::NotFinite);
        }
        if !self.min_curve_snr_db.is_finite() || !self.full_curve_snr_db.is_finite() {
            return Err(EventError::NotFinite);
        }
        if self.full_curve_snr_db <= self.min_curve_snr_db {
            return Err(EventError::InvalidRate(self.full_curve_snr_db));
        }
        if self.min_harmonics == 0 || self.target_harmonics < self.min_harmonics {
            return Err(EventError::ZeroLength);
        }
        if !self.max_residual_autocorrelation.is_finite()
            || !(0.0..=1.0).contains(&self.max_residual_autocorrelation)
        {
            return Err(EventError::InvalidRate(self.max_residual_autocorrelation));
        }
        if !self.max_covariance_inflation.is_finite() || self.max_covariance_inflation < 1.0 {
            return Err(EventError::InvalidRate(self.max_covariance_inflation));
        }
        if !self.retention_score.is_finite() || !(0.0..=1.0).contains(&self.retention_score) {
            return Err(EventError::InvalidRate(self.retention_score));
        }
        Ok(())
    }
}

/// Why an event failed the quality gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityReason {
    /// The curve SNR is below the configured minimum.
    LowSnr,
    /// Too few harmonics are visible in the map.
    FewHarmonics,
    /// The fit residuals are more correlated than allowed.
    ResidualAutocorrelation,
    /// The covariance is inflated beyond the allowed factor.
    CovarianceInflation,
    /// The combined score is below the retention cut.
    LowScore,
}

impl core::fmt::Display for QualityReason {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::LowSnr => "curve SNR below the minimum",
            Self::FewHarmonics => "too few visible harmonics",
            Self::ResidualAutocorrelation => "residual autocorrelation above the maximum",
            Self::CovarianceInflation => "covariance inflation above the maximum",
            Self::LowScore => "combined score below the retention cut",
        };
        formatter.write_str(text)
    }
}

/// The diagnostics and verdict for one fitted event.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityAssessment {
    /// Combined score in `[0, 1]`.
    pub quality: EventQuality,
    /// Whether the event survives every gate.
    pub retained: bool,
    /// Failures in a fixed order; empty when the event is retained.
    pub reasons: Vec<QualityReason>,
    /// Mean whitened curve SNR in decibels, or negative infinity when no
    /// sample falls inside the map.
    pub curve_snr_db: f64,
    /// Lag-one residual autocorrelation carried by the fit, in `[-1, 1]`.
    pub residual_autocorrelation: f64,
    /// Covariance scale carried by the fit; one means no inflation.
    pub covariance_inflation: f64,
    /// Harmonics whose curve had at least one in-band sample.
    pub harmonic_count: usize,
}

impl QualityAssessment {
    /// One-line explanation for the report: retained, or why it was dropped.
    pub fn explanation(&self) -> String {
        if self.reasons.is_empty() {
            return format!("retained with quality {:.3}", self.quality.score);
        }
        let reasons: Vec<String> = self.reasons.iter().map(ToString::to_string).collect();
        format!(
            "dropped with quality {:.3}: {}",
            self.quality.score,
            reasons.join("; ")
        )
    }
}

/// Score one fitted event and decide whether to keep it.
///
/// The curve is sampled with the same interpolation as the direct estimator,
/// so the score measures the evidence the fit actually claims. An event is
/// retained when no hard gate fires and the mean of the four components reaches
/// [`QualityConfig::retention_score`]; otherwise every failed reason is listed.
///
/// # Errors
///
/// Returns [`EventError::EmptyMap`] for a map without frames or bins,
/// [`EventError::NotFinite`] for a non-finite bin width, covariance scale or
/// autocorrelation, [`EventError::ZeroLength`] for an empty harmonic list, and
/// [`EventError::InvalidHarmonic`] for harmonic zero.
pub fn assess(
    map: &ZMap,
    report: &FitReport,
    config: &QualityConfig,
) -> Result<QualityAssessment, EventError> {
    validate_inputs(map, report, config)?;

    let (curve_snr_db, harmonic_count) = curve_statistics(map, report, config);
    let residual_autocorrelation = report.residual_autocorrelation.clamp(-1.0, 1.0);
    let covariance_inflation = report.covariance_scale;
    let components = score_components(
        curve_snr_db,
        harmonic_count,
        residual_autocorrelation,
        covariance_inflation,
        config,
    );
    let score = 0.25 * components.iter().sum::<f64>();
    let quality = EventQuality::from_score(score)?;
    let reasons = gate_reasons(
        curve_snr_db,
        harmonic_count,
        residual_autocorrelation,
        covariance_inflation,
        quality.score,
        config,
    );
    let retained = reasons.is_empty();

    Ok(QualityAssessment {
        quality,
        retained,
        reasons,
        curve_snr_db,
        residual_autocorrelation,
        covariance_inflation,
        harmonic_count,
    })
}

/// Reject inputs that cannot produce a meaningful score.
fn validate_inputs(
    map: &ZMap,
    report: &FitReport,
    config: &QualityConfig,
) -> Result<(), EventError> {
    config.validate()?;
    if map.frame_count() == 0 || map.bin_count() == 0 {
        return Err(EventError::EmptyMap);
    }
    if !map.config().bin_width_hz().is_finite() || map.config().bin_width_hz() <= 0.0 {
        return Err(EventError::NotFinite);
    }
    if report.harmonics.is_empty() {
        return Err(EventError::ZeroLength);
    }
    for &harmonic in &report.harmonics {
        if harmonic == 0 {
            return Err(EventError::InvalidHarmonic(harmonic));
        }
    }
    if !report.covariance_scale.is_finite() || report.covariance_scale < 0.0 {
        return Err(EventError::NotFinite);
    }
    if !report.residual_autocorrelation.is_finite() {
        return Err(EventError::NotFinite);
    }
    Ok(())
}

/// Mean whitened power along the fitted curve, as SNR in decibels, and the
/// number of harmonics with at least one in-band sample.
fn curve_statistics(map: &ZMap, report: &FitReport, config: &QualityConfig) -> (f64, usize) {
    let parameters = Parameters::from_hypothesis(&EventHypothesis {
        heard_at: report.fit.heard_at,
        fundamental_hz: Hertz(report.fit.fundamental_hz.0),
        speed_m_per_s: report.fit.speed_m_per_s,
        closest_m: report.fit.closest_m.0,
    });
    let curves = sampling::curve_z(
        map,
        &parameters,
        &report.harmonics,
        config.sound_speed_m_per_s,
    );

    let mut sample_count = 0_usize;
    let mut power_sum = 0.0_f64;
    let mut harmonic_count = 0_usize;
    for values in &curves {
        if values.is_empty() {
            continue;
        }
        harmonic_count = harmonic_count.saturating_add(1);
        for &z in values {
            power_sum += z * z;
            sample_count = sample_count.saturating_add(1);
        }
    }
    let curve_snr_db = if sample_count == 0 {
        f64::NEG_INFINITY
    } else {
        10.0 * (power_sum / sample_count as f64).log10()
    };
    (curve_snr_db, harmonic_count)
}

/// The four bounded components whose mean is the event score.
fn score_components(
    curve_snr_db: f64,
    harmonic_count: usize,
    residual_autocorrelation: f64,
    covariance_inflation: f64,
    config: &QualityConfig,
) -> [f64; 4] {
    let snr_span = config.full_curve_snr_db - config.min_curve_snr_db;
    let snr_component = if curve_snr_db.is_finite() {
        ((curve_snr_db - config.min_curve_snr_db) / snr_span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let harmonic_component =
        (harmonic_count as f64 / config.target_harmonics as f64).clamp(0.0, 1.0);
    let autocorrelation_excess = residual_autocorrelation.max(0.0);
    let autocorrelation_component = if config.max_residual_autocorrelation <= 0.0 {
        if autocorrelation_excess <= 0.0 {
            1.0
        } else {
            0.0
        }
    } else {
        (1.0 - autocorrelation_excess / config.max_residual_autocorrelation).clamp(0.0, 1.0)
    };
    let inflation_component = if config.max_covariance_inflation <= 1.0 {
        if covariance_inflation <= 1.0 {
            1.0
        } else {
            0.0
        }
    } else {
        ((config.max_covariance_inflation - covariance_inflation)
            / (config.max_covariance_inflation - 1.0))
            .clamp(0.0, 1.0)
    };
    [
        snr_component,
        harmonic_component,
        autocorrelation_component,
        inflation_component,
    ]
}

/// Hard gates followed by the combined-score cut, in a fixed order.
fn gate_reasons(
    curve_snr_db: f64,
    harmonic_count: usize,
    residual_autocorrelation: f64,
    covariance_inflation: f64,
    score: f64,
    config: &QualityConfig,
) -> Vec<QualityReason> {
    let mut reasons = Vec::new();
    if curve_snr_db < config.min_curve_snr_db {
        reasons.push(QualityReason::LowSnr);
    }
    if harmonic_count < config.min_harmonics {
        reasons.push(QualityReason::FewHarmonics);
    }
    if residual_autocorrelation > config.max_residual_autocorrelation {
        reasons.push(QualityReason::ResidualAutocorrelation);
    }
    if covariance_inflation > config.max_covariance_inflation {
        reasons.push(QualityReason::CovarianceInflation);
    }
    if score < config.retention_score {
        reasons.push(QualityReason::LowScore);
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::{QualityConfig, QualityReason, assess};
    use crate::detect::EventHypothesis;
    use crate::direct::{DirectConfig, fit_event_direct};
    use crate::error::EventError;
    use crate::fit::{EventFit, FitMethod, FitReport, Parameters, predicted_hz_at};
    use core_types::{AudioSeconds, EventId, Hertz, Metres};
    use dsp::{StftConfig, ZMap};

    /// Speed of sound for every fixture, metres per second.
    const SOUND_SPEED: f64 = 343.0;
    /// Standard deviation of the injected ridge in frequency bins.
    const RIDGE_SIGMA_BINS: f64 = 1.5;
    /// Frames in the synthetic survey map.
    const FRAMES: usize = 250;
    /// Analysis sample rate, hertz.
    const SAMPLE_RATE_HZ: f64 = 4_000.0;
    /// STFT hop in samples.
    const HOP: usize = 64;

    fn truth() -> EventHypothesis {
        EventHypothesis {
            heard_at: AudioSeconds(2.0),
            fundamental_hz: Hertz(180.0),
            speed_m_per_s: 120.0,
            closest_m: 800.0,
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

    /// A z-map with seeded noise and an optional emission-time ridge.
    fn signal_map(state: &mut u32, amplitude: f64) -> ZMap {
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
            if amplitude <= 0.0 {
                continue;
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
                    let signal = amplitude * (-0.5 * offset * offset).exp();
                    values[frame * bins + bin] += signal as f32;
                }
            }
        }
        ZMap::from_parts(stft, times, values).expect("map shape")
    }

    /// A report with the truth parameters and no estimator run.
    fn report(
        covariance_scale: f64,
        residual_autocorrelation: f64,
        harmonics: Vec<u32>,
    ) -> FitReport {
        let truth = truth();
        FitReport {
            fit: EventFit {
                event_id: EventId::new_v7(),
                fundamental_hz: truth.fundamental_hz,
                speed_m_per_s: truth.speed_m_per_s,
                closest_m: Metres(truth.closest_m),
                heard_at: truth.heard_at,
                covariance: [[0.0; 4]; 4],
            },
            method: FitMethod::Direct,
            chi_squared: 0.0,
            degrees_of_freedom: 1,
            covariance_scale,
            residual_autocorrelation,
            robust_cost: 0.0,
            speed_closest_correlation: 0.0,
            speed_interval: None,
            closest_interval: None,
            iterations: 0,
            observations: 1,
            harmonics,
        }
    }

    fn config() -> QualityConfig {
        QualityConfig {
            sound_speed_m_per_s: SOUND_SPEED,
            ..QualityConfig::default()
        }
    }

    #[test]
    fn fr_dm_06_a_strong_event_is_retained() {
        let mut state = 0x2468_ace0_u32;
        let map = signal_map(&mut state, 4.0);
        let direct = DirectConfig {
            sound_speed_m_per_s: SOUND_SPEED,
            ..DirectConfig::default()
        };
        let fit = fit_event_direct(&map, EventId::new_v7(), &truth(), &[1, 2], &direct)
            .expect("direct fit");

        let assessment = assess(&map, &fit, &config()).expect("quality assessment");
        assert!(assessment.retained, "{:?}", assessment.reasons);
        assert_eq!(assessment.reasons, Vec::new());
        assert_eq!(assessment.harmonic_count, 2);
        assert!(
            assessment.curve_snr_db > config().min_curve_snr_db,
            "curve SNR {}",
            assessment.curve_snr_db
        );
        assert!(
            assessment.quality.score >= config().retention_score,
            "score {}",
            assessment.quality.score
        );
        assert!(assessment.explanation().starts_with("retained"));
    }

    #[test]
    fn fr_dm_06_noise_alone_is_dropped_with_a_reason() {
        let mut state = 0x1357_9bdf_u32;
        let map = signal_map(&mut state, 0.0);
        let fit = report(1.0, 0.0, vec![1, 2]);

        let assessment = assess(&map, &fit, &config()).expect("quality assessment");
        assert!(!assessment.retained);
        assert!(
            assessment.reasons.contains(&QualityReason::LowSnr),
            "{:?}",
            assessment.reasons
        );
        assert!(
            assessment.curve_snr_db < config().min_curve_snr_db,
            "curve SNR {}",
            assessment.curve_snr_db
        );
        assert!(assessment.explanation().contains("curve SNR"));
    }

    #[test]
    fn fr_dm_06_inflation_and_autocorrelation_are_rejected() {
        let mut state = 0x0f1e_2d3c_u32;
        let map = signal_map(&mut state, 4.0);
        let fit = report(100.0, 0.9, vec![1, 2]);

        let assessment = assess(&map, &fit, &config()).expect("quality assessment");
        assert!(!assessment.retained);
        assert!(
            assessment
                .reasons
                .contains(&QualityReason::ResidualAutocorrelation),
            "{:?}",
            assessment.reasons
        );
        assert!(
            assessment
                .reasons
                .contains(&QualityReason::CovarianceInflation),
            "{:?}",
            assessment.reasons
        );
        assert_eq!(assessment.covariance_inflation, 100.0);
        assert_eq!(assessment.residual_autocorrelation, 0.9);
    }

    #[test]
    fn fr_dm_06_assessment_is_deterministic() {
        let mut state = 0x0a0b_0c0d_u32;
        let map = signal_map(&mut state, 4.0);
        let fit = report(1.0, 0.0, vec![1, 2]);
        let first = assess(&map, &fit, &config()).expect("first assessment");
        let second = assess(&map, &fit, &config()).expect("second assessment");
        assert_eq!(first, second);
    }

    #[test]
    fn fr_dm_06_invalid_config_is_rejected() {
        let mut state = 0x1111_2222_u32;
        let map = signal_map(&mut state, 4.0);
        let fit = report(1.0, 0.0, vec![1]);
        let base = QualityConfig {
            sound_speed_m_per_s: SOUND_SPEED,
            ..QualityConfig::default()
        };

        let mut bad_snr = base;
        bad_snr.full_curve_snr_db = bad_snr.min_curve_snr_db;
        assert!(matches!(
            bad_snr.validate(),
            Err(EventError::InvalidRate(_))
        ));

        let mut bad_autocorrelation = base;
        bad_autocorrelation.max_residual_autocorrelation = 1.5;
        assert!(matches!(
            bad_autocorrelation.validate(),
            Err(EventError::InvalidRate(_))
        ));

        let mut bad_harmonics = base;
        bad_harmonics.min_harmonics = 0;
        assert!(matches!(
            bad_harmonics.validate(),
            Err(EventError::ZeroLength)
        ));

        let mut bad_score = base;
        bad_score.retention_score = 2.0;
        assert!(matches!(
            bad_score.validate(),
            Err(EventError::InvalidRate(_))
        ));

        let mut bad_speed = base;
        bad_speed.sound_speed_m_per_s = 0.0;
        assert!(matches!(bad_speed.validate(), Err(EventError::NotFinite)));

        assert!(assess(&map, &fit, &bad_speed).is_err());
    }
}
