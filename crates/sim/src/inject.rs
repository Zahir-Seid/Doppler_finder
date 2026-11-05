//! Degraded ADS-B feed and input-error ranking (FR-SIM-02, SDD section 7).
//!
//! SDD section 7 makes one scenario and one seed produce a synthetic ADS-B
//! feed alongside the audio, degraded by a stated error profile. This module
//! builds that feed for a straight, level pass and measures what each error
//! source alone does to the three quantities the localization stage reads off
//! a track: the heard closest-approach time, the closest distance and the
//! ground speed.
//!
//! The ranking below is a ranking of **input** error. It says which injected
//! source moves the observables most before an estimator sees them; it is not
//! the G2 error budget, which needs the estimators of `events` and `search` to
//! measure position error (Q-13).
//!
//! Terrain, geoid and manoeuvre sources need reference data the simulator does
//! not carry yet, so they are not injected here.

use core_types::LatLon;
use geodesy::{Enu, LocalFrame};
use serde::{Deserialize, Serialize};

use crate::error::SimError;
use crate::rng::NormalSource;
use crate::scenario::{AircraftPass, Scenario};

/// One position as an ADS-B archive would report it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPoint {
    /// Callsign of the reported aircraft.
    pub callsign: String,
    /// Reported timestamp, seconds from the recording start.
    pub t_s: f64,
    /// Reported geodetic position.
    pub position: LatLon,
    /// Reported altitude in metres, after the datum offset and quantization.
    pub altitude_m: f64,
}

/// The synthetic feed of one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    points: Vec<TrackPoint>,
}

impl Feed {
    /// The points, oldest first.
    pub fn points(&self) -> &[TrackPoint] {
        &self.points
    }

    /// Number of reported points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Return `true` when the feed holds no points.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// Magnitude of each injected error source.
///
/// [`ErrorProfile::perfect`] injects nothing; every field is then a deviation
/// from that, in the unit named on the field. A zero sampling interval is not
/// allowed because a feed with no sample rate cannot be generated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErrorProfile {
    /// Interval between samples in seconds, about 1 s for the archives.
    pub sampling_interval_s: f64,
    /// Constant delivery latency in seconds; timestamps are reported later.
    pub latency_s: f64,
    /// Standard deviation of the per-sample timestamp jitter, seconds.
    pub time_jitter_s: f64,
    /// Reported position rounding in degrees; zero disables rounding.
    pub position_quantization_deg: f64,
    /// Reported altitude rounding in metres; zero disables rounding.
    pub altitude_quantization_m: f64,
    /// Barometric-minus-geometric altitude offset in metres.
    pub altitude_datum_offset_m: f64,
}

impl ErrorProfile {
    /// A profile that injects nothing but still samples once per second.
    pub const fn perfect() -> Self {
        Self {
            sampling_interval_s: 1.0,
            latency_s: 0.0,
            time_jitter_s: 0.0,
            position_quantization_deg: 0.0,
            altitude_quantization_m: 0.0,
            altitude_datum_offset_m: 0.0,
        }
    }

    /// Check every field.
    ///
    /// Returns [`SimError::InvalidProfile`] for a non-positive or non-finite
    /// interval and for any negative or non-finite magnitude.
    pub fn validate(&self) -> Result<(), SimError> {
        let sources = [
            ("sampling_interval_s", self.sampling_interval_s),
            ("latency_s", self.latency_s),
            ("time_jitter_s", self.time_jitter_s),
            ("position_quantization_deg", self.position_quantization_deg),
            ("altitude_quantization_m", self.altitude_quantization_m),
            ("altitude_datum_offset_m", self.altitude_datum_offset_m),
        ];
        for (field, value) in sources {
            if !value.is_finite() {
                return Err(SimError::InvalidProfile { field, value });
            }
            if field == "sampling_interval_s" && value <= 0.0 {
                return Err(SimError::InvalidProfile { field, value });
            }
            if field != "sampling_interval_s" && value < 0.0 {
                return Err(SimError::InvalidProfile { field, value });
            }
        }
        Ok(())
    }
}

/// One injected error source, named for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSource {
    /// Coarse sampling of the true path.
    Sampling,
    /// Constant timestamp latency.
    Latency,
    /// Timestamp jitter.
    TimeJitter,
    /// Latitude and longitude rounding.
    PositionQuantization,
    /// Altitude rounding.
    AltitudeQuantization,
    /// Barometric-versus-geometric altitude offset.
    AltitudeDatum,
}

impl ErrorSource {
    /// Every source the simulator can inject, in a fixed order.
    pub const ALL: [Self; 6] = [
        Self::Sampling,
        Self::Latency,
        Self::TimeJitter,
        Self::PositionQuantization,
        Self::AltitudeQuantization,
        Self::AltitudeDatum,
    ];

    /// Stable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sampling => "sampling",
            Self::Latency => "latency",
            Self::TimeJitter => "time_jitter",
            Self::PositionQuantization => "position_quantization",
            Self::AltitudeQuantization => "altitude_quantization",
            Self::AltitudeDatum => "altitude_datum",
        }
    }

    /// A perfect profile with only this source taken from `profile`.
    fn only(self, profile: &ErrorProfile) -> ErrorProfile {
        let mut single = ErrorProfile::perfect();
        single.sampling_interval_s = profile.sampling_interval_s;
        match self {
            Self::Sampling => {}
            Self::Latency => single.latency_s = profile.latency_s,
            Self::TimeJitter => single.time_jitter_s = profile.time_jitter_s,
            Self::PositionQuantization => {
                single.position_quantization_deg = profile.position_quantization_deg;
            }
            Self::AltitudeQuantization => {
                single.altitude_quantization_m = profile.altitude_quantization_m;
            }
            Self::AltitudeDatum => {
                single.altitude_datum_offset_m = profile.altitude_datum_offset_m;
            }
        }
        single
    }
}

/// The three track observables localization reads, on the audio clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observable {
    /// Time of the nearest reported sample, seconds from the recording start.
    pub heard_time_s: f64,
    /// Horizontal distance at that sample, metres.
    pub closest_distance_m: f64,
    /// Mean ground speed between the first and last sample, metres per second.
    pub speed_m_per_s: f64,
}

/// Impact of one injected source on the observables.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceImpact {
    /// The injected source.
    pub source: ErrorSource,
    /// Absolute speed error, metres per second.
    pub speed_error_m_per_s: f64,
    /// Absolute closest-distance error, metres.
    pub closest_distance_error_m: f64,
    /// Absolute heard-time error, seconds.
    pub heard_time_error_s: f64,
}

/// Build the degraded feed for every pass of a scenario.
///
/// Each pass is sampled over the whole recording at the profile's interval,
/// its position rounded and its altitude offset as the profile states, then
/// the timestamps are shifted by the latency and jittered from the scenario
/// seed. The result is ordered by reported time.
///
/// Returns [`SimError`] for an invalid scenario or profile, for a pass below
/// the ear height and for a geodesy failure.
pub fn degraded_feed(scenario: &Scenario, profile: &ErrorProfile) -> Result<Feed, SimError> {
    scenario.validate()?;
    profile.validate()?;
    let frame = LocalFrame::wgs84(scenario.observer.position)?;
    let mut source = NormalSource::seeded(scenario.seed);
    let mut points = Vec::new();

    for pass in &scenario.passes {
        let mut sample_index = 0_u64;
        loop {
            let true_time_s = sample_index as f64 * profile.sampling_interval_s;
            if true_time_s > scenario.duration_s {
                break;
            }
            let enu = aircraft_enu(pass, scenario.observer.ear_height_m, true_time_s)?;
            let (position, _) = frame.enu_to_geodetic(enu)?;
            let rounded = LatLon::new(
                quantize(position.lat_deg.0, profile.position_quantization_deg),
                quantize(position.lon_deg.0, profile.position_quantization_deg),
            );
            rounded.validate().map_err(|_| SimError::InvalidObserver)?;
            let altitude_m = quantize(
                pass.altitude_m + profile.altitude_datum_offset_m,
                profile.altitude_quantization_m,
            );
            let jitter = profile.time_jitter_s * source.standard_normal();
            points.push(TrackPoint {
                callsign: pass.callsign.clone(),
                t_s: true_time_s + profile.latency_s + jitter,
                position: rounded,
                altitude_m,
            });
            sample_index += 1;
        }
    }

    points.sort_by(|left, right| left.t_s.total_cmp(&right.t_s));
    Ok(Feed { points })
}

/// Read the observables of one callsign off a feed.
///
/// The closest distance is the smallest horizontal distance to the observer
/// among the reported samples, so a coarse interval biases it outwards; the
/// speed is the mean between the first and last sample of the callsign.
///
/// Returns [`SimError::EmptyFeed`] when the callsign has fewer than two
/// points, and a geodesy error for an unreadable position.
pub fn observe(feed: &Feed, scenario: &Scenario, callsign: &str) -> Result<Observable, SimError> {
    let frame = LocalFrame::wgs84(scenario.observer.position)?;
    let mut closest_distance_m = f64::INFINITY;
    let mut heard_time_s = 0.0;
    let mut first: Option<(f64, Enu)> = None;
    let mut last: Option<(f64, Enu)> = None;

    for point in feed
        .points
        .iter()
        .filter(|point| point.callsign == callsign)
    {
        let enu = frame.to_enu(point.position, 0.0)?;
        let distance = enu.horizontal_distance_m();
        if distance < closest_distance_m {
            closest_distance_m = distance;
            heard_time_s = point.t_s;
        }
        if first.is_none() {
            first = Some((point.t_s, enu));
        }
        last = Some((point.t_s, enu));
    }

    let (Some((first_time, first_enu)), Some((last_time, last_enu))) = (first, last) else {
        return Err(SimError::EmptyFeed);
    };
    let interval_s = last_time - first_time;
    if !interval_s.is_finite() || interval_s <= 0.0 {
        return Err(SimError::EmptyFeed);
    }

    let path_m = (last_enu.east_m - first_enu.east_m).hypot(last_enu.north_m - first_enu.north_m);
    Ok(Observable {
        heard_time_s,
        closest_distance_m,
        speed_m_per_s: path_m / interval_s,
    })
}

/// The observables the scenario states, with no injected error.
///
/// The time is the **geometric** closest approach, the instant the aircraft is
/// nearest the observer. A feed carries no acoustics, so the propagation delay
/// that turns this into the heard time in the truth file belongs to the sound
/// model, not to the track (SDD section 6.1); `search` adds it when it predicts
/// what a listener would hear.
///
/// Returns [`SimError::UnknownCallsign`] when no pass carries the callsign.
pub fn truth_observable(scenario: &Scenario, callsign: &str) -> Result<Observable, SimError> {
    scenario.validate()?;
    let pass = scenario
        .passes
        .iter()
        .find(|pass| pass.callsign == callsign)
        .ok_or_else(|| SimError::UnknownCallsign(callsign.to_owned()))?;
    let height_m = pass.altitude_m - scenario.observer.ear_height_m;
    if !height_m.is_finite() || height_m <= 0.0 {
        return Err(SimError::PassBelowObserver { index: 0 });
    }
    Ok(Observable {
        heard_time_s: pass.offset_s,
        closest_distance_m: pass.closest_approach_m,
        speed_m_per_s: pass.velocity_m_per_s,
    })
}

/// Rank the injected sources by how far each moves the closest distance.
///
/// Each source is applied alone at the magnitude `profile` gives it, the feed
/// is rebuilt and read back, and the difference from the truth is recorded.
/// The report is sorted by closest-distance error, then heard-time error, then
/// speed error, with the fixed source order breaking ties. The result is a
/// statement about the inputs, not about final position accuracy (Q-13).
///
/// Returns the errors of [`degraded_feed`] and [`truth_observable`].
pub fn sensitivity(
    scenario: &Scenario,
    profile: &ErrorProfile,
) -> Result<Vec<SourceImpact>, SimError> {
    profile.validate()?;
    let callsign = scenario
        .passes
        .first()
        .ok_or(SimError::NoPasses)?
        .callsign
        .clone();
    let truth = truth_observable(scenario, &callsign)?;

    let mut impacts = Vec::with_capacity(ErrorSource::ALL.len());
    for source in ErrorSource::ALL {
        let feed = degraded_feed(scenario, &source.only(profile))?;
        let measured = observe(&feed, scenario, &callsign)?;
        impacts.push(SourceImpact {
            source,
            speed_error_m_per_s: (measured.speed_m_per_s - truth.speed_m_per_s).abs(),
            closest_distance_error_m: (measured.closest_distance_m - truth.closest_distance_m)
                .abs(),
            heard_time_error_s: (measured.heard_time_s - truth.heard_time_s).abs(),
        });
    }
    impacts.sort_by(|left, right| {
        right
            .closest_distance_error_m
            .total_cmp(&left.closest_distance_error_m)
            .then(right.heard_time_error_s.total_cmp(&left.heard_time_error_s))
            .then(
                right
                    .speed_error_m_per_s
                    .total_cmp(&left.speed_error_m_per_s),
            )
            .then(left.source.cmp(&right.source))
    });
    Ok(impacts)
}

/// Aircraft position in the observer's local frame at one time.
fn aircraft_enu(pass: &AircraftPass, ear_height_m: f64, t_s: f64) -> Result<Enu, SimError> {
    let height_m = pass.altitude_m - ear_height_m;
    if !height_m.is_finite() || height_m <= 0.0 {
        return Err(SimError::PassBelowObserver { index: 0 });
    }
    let heading = pass.heading_deg.to_radians();
    let along_track_m = pass.velocity_m_per_s * (t_s - pass.offset_s);
    let east_m = heading.cos() * pass.closest_approach_m + heading.sin() * along_track_m;
    let north_m = -heading.sin() * pass.closest_approach_m + heading.cos() * along_track_m;
    Ok(Enu {
        east_m,
        north_m,
        up_m: height_m,
    })
}

/// Round to `quantum`, leaving the value alone when the quantum is zero.
fn quantize(value: f64, quantum: f64) -> f64 {
    if quantum <= 0.0 {
        return value;
    }
    (value / quantum).round() * quantum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Observer;

    /// Position quantum of the common ADS-B message format: 1e-5 degrees is
    /// about 1.1 m at this latitude.
    const POSITION_QUANTUM_DEG: f64 = 1e-5;
    /// Altitude quantum of 25 ft, the common barometric resolution.
    const ALTITUDE_QUANTUM_M: f64 = 7.62;

    fn pass() -> AircraftPass {
        AircraftPass {
            callsign: "TEST123".to_owned(),
            velocity_m_per_s: 90.0,
            closest_approach_m: 800.0,
            // Deliberately off the one-second sampling grid, otherwise the
            // nearest sample lands exactly on closest approach and sampling
            // costs nothing.
            offset_s: 30.37,
            altitude_m: 600.0,
            heading_deg: 90.0,
            fundamental_hz: 100.0,
            harmonics: vec![1.0],
        }
    }

    fn scenario() -> Scenario {
        Scenario {
            name: "feed".to_owned(),
            seed: 17,
            duration_s: 60.0,
            observer: Observer {
                position: LatLon::new(52.0, 13.0),
                ear_height_m: 1.7,
            },
            passes: vec![pass()],
        }
    }

    #[test]
    fn perfect_feed_reproduces_the_track() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.sampling_interval_s = 0.5;
        let feed = degraded_feed(&scenario, &profile).expect("valid");
        assert_eq!(feed.len(), 121);
        let measured = observe(&feed, &scenario, "TEST123").expect("observes");
        let truth = truth_observable(&scenario, "TEST123").expect("truth");
        // Half-second sampling puts the nearest sample within 0.25 s of the
        // true closest approach, at most 22.5 m along track, so the distance
        // is high by about (22.5)^2 / (2 * 800) = 0.32 m plus the geodesy
        // round trip, which the frame keeps below a millimetre.
        assert!(
            (measured.closest_distance_m - truth.closest_distance_m).abs() < 1.0,
            "{} vs {}",
            measured.closest_distance_m,
            truth.closest_distance_m
        );
        assert!((measured.speed_m_per_s - truth.speed_m_per_s).abs() < 0.5);
        assert!((measured.heard_time_s - truth.heard_time_s).abs() < 0.3);
    }

    #[test]
    fn latency_moves_the_reported_closest_approach_later() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.sampling_interval_s = 0.05;
        profile.latency_s = 2.0;
        let feed = degraded_feed(&scenario, &profile).expect("valid");
        let measured = observe(&feed, &scenario, "TEST123").expect("observes");
        let truth = truth_observable(&scenario, "TEST123").expect("truth");
        // A constant shift moves every timestamp by the same amount, so the
        // horizontal geometry and the speed are untouched. The nearest sample
        // is within 0.025 s of closest approach, which is 2.25 m along track.
        assert!((measured.closest_distance_m - truth.closest_distance_m).abs() < 1.0);
        assert!((measured.speed_m_per_s - truth.speed_m_per_s).abs() < 0.5);
        assert!(
            (measured.heard_time_s - truth.heard_time_s - 2.0).abs() < 0.1,
            "heard time shifted by {} s",
            measured.heard_time_s - truth.heard_time_s
        );
    }

    #[test]
    fn quantization_stays_within_half_a_step() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.position_quantization_deg = POSITION_QUANTUM_DEG;
        let feed = degraded_feed(&scenario, &profile).expect("valid");
        let measured = observe(&feed, &scenario, "TEST123").expect("observes");
        let truth = truth_observable(&scenario, "TEST123").expect("truth");
        // Half a step is 5e-6 degrees, about 0.55 m in latitude and 0.35 m in
        // longitude at 52 N, so a metre covers the rounding in both axes.
        assert!(
            (measured.closest_distance_m - truth.closest_distance_m).abs() < 1.0,
            "got {}",
            measured.closest_distance_m - truth.closest_distance_m
        );
    }

    #[test]
    fn coarse_sampling_biases_the_distance_outwards() {
        let scenario = scenario();
        let mut coarse = ErrorProfile::perfect();
        coarse.sampling_interval_s = 1.0;
        let mut fine = ErrorProfile::perfect();
        fine.sampling_interval_s = 0.05;

        let coarse_feed = degraded_feed(&scenario, &coarse).expect("valid");
        let fine_feed = degraded_feed(&scenario, &fine).expect("valid");
        let coarse_measured = observe(&coarse_feed, &scenario, "TEST123").expect("observes");
        let fine_measured = observe(&fine_feed, &scenario, "TEST123").expect("observes");
        // With one sample per second the nearest sample is 0.37 s along track
        // from the truth, adding about (90 * 0.37)^2 / (2 * 800) = 0.69 m.
        let bias = coarse_measured.closest_distance_m - fine_measured.closest_distance_m;
        assert!(bias > 0.3, "coarse must read farther out, got {bias} m");
        assert!(bias < 5.0, "the bias should be sub-metre, got {bias} m");
    }

    #[test]
    fn altitude_errors_leave_horizontal_observables_alone() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.altitude_datum_offset_m = 30.0;
        profile.altitude_quantization_m = ALTITUDE_QUANTUM_M;
        let feed = degraded_feed(&scenario, &profile).expect("valid");
        let point = feed.points().first().expect("a point");
        // 600 + 30 = 630 m rounded to 7.62 m is 629.7 m, and the datum error
        // never enters a horizontal distance.
        assert!((point.altitude_m - 629.7).abs() < ALTITUDE_QUANTUM_M);
        let measured = observe(&feed, &scenario, "TEST123").expect("observes");
        let truth = truth_observable(&scenario, "TEST123").expect("truth");
        assert!((measured.closest_distance_m - truth.closest_distance_m).abs() < 1.0);
    }

    #[test]
    fn feed_is_deterministic_for_a_fixed_seed() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.time_jitter_s = 0.2;
        let first = degraded_feed(&scenario, &profile).expect("valid");
        let second = degraded_feed(&scenario, &profile).expect("valid");
        assert_eq!(first, second);
        assert!(
            first
                .points()
                .iter()
                .any(|point| (point.t_s.fract() - 0.0).abs() > 1e-6),
            "jitter should move at least one timestamp off the grid"
        );
    }

    #[test]
    fn sensitivity_ranks_every_source_and_puts_sampling_first() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.latency_s = 2.0;
        profile.time_jitter_s = 0.1;
        // One part in a million of position is about 0.1 m, well below the
        // 0.69 m that one-second sampling adds, so the ranking has a clear
        // order to find.
        profile.position_quantization_deg = 1e-6;
        profile.altitude_quantization_m = ALTITUDE_QUANTUM_M;
        profile.altitude_datum_offset_m = 30.0;
        let impacts = sensitivity(&scenario, &profile).expect("valid profile");
        assert_eq!(impacts.len(), ErrorSource::ALL.len());
        for pair in impacts.windows(2) {
            assert!(
                pair[0].closest_distance_error_m >= pair[1].closest_distance_error_m,
                "report must be ordered"
            );
        }
        let sampling = impacts
            .iter()
            .find(|impact| impact.source == ErrorSource::Sampling)
            .expect("sampling is in the report");
        // The nearest one-second sample is 0.37 s along track from closest
        // approach, which costs about 0.69 m in distance.
        assert!(
            (sampling.closest_distance_error_m - 0.69).abs() < 0.3,
            "got {} m",
            sampling.closest_distance_error_m
        );
        // A constant latency moves timestamps, not positions, so it ties with
        // sampling on distance and wins the tie-break on heard time.
        assert_eq!(
            impacts.first().map(|impact| impact.source),
            Some(ErrorSource::Latency),
            "the distance tie must break on the time error"
        );
        // Latency shifts the heard time by nearly its full magnitude, less the
        // fraction of a second the nearest sample already sits before closest
        // approach.
        let latency = impacts
            .iter()
            .find(|impact| impact.source == ErrorSource::Latency)
            .expect("latency is in the report");
        assert!((latency.heard_time_error_s - 1.63).abs() < 0.3);
        assert!(
            impacts
                .iter()
                .all(|impact| impact.heard_time_error_s <= latency.heard_time_error_s + 1e-9)
        );
    }

    #[test]
    fn impossible_profiles_are_rejected() {
        let scenario = scenario();
        let mut profile = ErrorProfile::perfect();
        profile.sampling_interval_s = 0.0;
        assert!(matches!(
            degraded_feed(&scenario, &profile),
            Err(SimError::InvalidProfile { .. })
        ));
        profile = ErrorProfile::perfect();
        profile.latency_s = -1.0;
        assert!(matches!(
            degraded_feed(&scenario, &profile),
            Err(SimError::InvalidProfile { .. })
        ));
        assert!(matches!(
            observe(&Feed { points: Vec::new() }, &scenario, "TEST123"),
            Err(SimError::EmptyFeed)
        ));
        assert!(matches!(
            truth_observable(&scenario, "NOBODY"),
            Err(SimError::UnknownCallsign(_))
        ));
    }
}
