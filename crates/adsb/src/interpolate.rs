//! Cubic Hermite interpolation between track samples (SDD section 4).
//!
//! The light-time solver needs the aircraft position `p(t)` at arbitrary
//! emission times. Positions are interpolated as cubics in a local
//! East-North-Up frame anchored at the first sample, which keeps longitude
//! wraps and latitude distortion out of the cubic. Knot velocities come from
//! centred finite differences of the stored positions, so interpolation does
//! not depend on the optional reported velocity fields.
//!
//! [`interpolate`] keeps the [`TrackSample`] entry point used by replay code;
//! the solver seam is [`TrackInterpolator::position_ecef`], which returns the
//! position in the same ECEF frame `acoustics::solve_emission_time` expects.

use core_types::{LatLon, UtcMillis};
use geodesy::{Ecef, Enu, LocalFrame};

use crate::error::AdsbError;
use crate::tracks::{FlightTrack, TrackSample};

/// One knot of the interpolating cubic: time, position and velocity.
#[derive(Debug, Clone, Copy)]
struct Knot {
    /// Seconds since the Unix epoch.
    unix_s: f64,
    /// Position in the track's local ENU frame, metres.
    position: Enu,
    /// Velocity in the track's local ENU frame, metres per second.
    velocity: Enu,
}

/// Cubic Hermite interpolator for one flight track.
///
/// Build once per track and evaluate many times; every evaluation is a binary
/// search plus one cubic. The interpolant passes exactly through every stored
/// sample and has continuous position and velocity.
#[derive(Debug)]
pub struct TrackInterpolator {
    /// Stored samples, kept for the fields Hermite does not model.
    samples: Vec<TrackSample>,
    /// Local frame the cubic lives in, anchored at the first sample.
    frame: LocalFrame,
    /// Strictly increasing knots in time order.
    knots: Vec<Knot>,
}

impl TrackInterpolator {
    /// Build an interpolator for `track`.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::InvalidTrack`] when the track fails validation,
    /// has fewer than two samples, or has a repeated timestamp, which a cubic
    /// cannot span. Returns [`AdsbError::Geodesy`] when a sample cannot be
    /// converted into the local frame.
    pub fn new(track: &FlightTrack) -> Result<Self, AdsbError> {
        track.validate()?;
        if track.samples.len() < 2 {
            return Err(AdsbError::InvalidTrack(
                "interpolation needs at least two samples".to_owned(),
            ));
        }
        let first = track
            .samples
            .first()
            .ok_or_else(|| AdsbError::InvalidTrack("track is empty".to_owned()))?;
        let frame = LocalFrame::wgs84_with_height(
            LatLon::new(first.lat_deg, first.lon_deg),
            first.geometric_altitude_m,
        )?;

        let mut knots: Vec<Knot> = Vec::with_capacity(track.samples.len());
        for sample in &track.samples {
            let unix_s = unix_seconds(sample.utc);
            if let Some(previous) = knots.last()
                && unix_s <= previous.unix_s
            {
                return Err(AdsbError::InvalidTrack(
                    "samples must have strictly increasing times".to_owned(),
                ));
            }
            let position = frame.to_enu(
                LatLon::new(sample.lat_deg, sample.lon_deg),
                sample.geometric_altitude_m,
            )?;
            knots.push(Knot {
                unix_s,
                position,
                velocity: Enu::default(),
            });
        }

        let mut interpolator = Self {
            samples: track.samples.clone(),
            frame,
            knots,
        };
        interpolator.fill_velocities()?;
        Ok(interpolator)
    }

    /// Number of stored samples.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Interpolated sample at a UTC time.
    ///
    /// Position comes from the cubic; barometric altitude, vertical rate,
    /// ground speed and track angle keep the linear semantics of
    /// [`interpolate`]. Returns `None` outside the track's time span.
    pub fn sample_at(&self, at: UtcMillis) -> Option<TrackSample> {
        let unix_s = unix_seconds(at);
        let (index, position, _) = self.state_at(unix_s)?;
        let (position_ll, height_m) = self.frame.enu_to_geodetic(position).ok()?;
        let first = self.samples.get(index)?;
        let second = self.samples.get(index + 1)?;
        let fraction = fraction_between(first.utc, second.utc, at)?;
        Some(TrackSample {
            utc: at,
            lat_deg: position_ll.lat_deg.0,
            lon_deg: position_ll.lon_deg.0,
            geometric_altitude_m: height_m,
            barometric_altitude_m: lerp_optional(
                first.barometric_altitude_m,
                second.barometric_altitude_m,
                fraction,
            ),
            ground_speed_m_per_s: first.ground_speed_m_per_s,
            track_deg: first.track_deg,
            vertical_rate_m_per_s: lerp_optional(
                first.vertical_rate_m_per_s,
                second.vertical_rate_m_per_s,
                fraction,
            ),
            latency_s: lerp_optional(first.latency_s, second.latency_s, fraction),
            flags: first.flags | second.flags | crate::tracks::FLAG_INTERPOLATED,
        })
    }

    /// Aircraft position in ECEF at `unix_time_s` seconds since the epoch.
    ///
    /// This is the seam `acoustics::solve_emission_time` plugs into: pass a
    /// closure `|t: Seconds| interp.position_ecef(t.0 + unix_epoch_offset)` or
    /// map the solver's seconds onto Unix time. Returns `None` outside the
    /// track's span.
    pub fn position_ecef(&self, unix_time_s: f64) -> Option<Ecef> {
        let (_, position, _) = self.state_at(unix_time_s)?;
        self.frame.from_enu(position).ok()
    }

    /// ENU velocity at `unix_time_s`, metres per second.
    pub(crate) fn velocity_at(&self, unix_time_s: f64) -> Option<Enu> {
        self.state_at(unix_time_s).map(|(_, _, velocity)| velocity)
    }

    /// Time span and interval list for closest-approach searches.
    pub(crate) fn intervals(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.knots.windows(2).filter_map(|pair| {
            let first = pair.first()?;
            let second = pair.get(1)?;
            Some((first.unix_s, second.unix_s))
        })
    }

    /// Locate the bracketing interval and evaluate the cubic.
    ///
    /// Returns the sample index of the interval's first knot, the ENU
    /// position and the ENU velocity, all at `unix_s`.
    fn state_at(&self, unix_s: f64) -> Option<(usize, Enu, Enu)> {
        if !unix_s.is_finite() || self.knots.len() < 2 {
            return None;
        }
        let first = self.knots.first()?;
        let last = self.knots.last()?;
        if unix_s < first.unix_s || unix_s > last.unix_s {
            return None;
        }
        // Knots are strictly increasing, so the interval index is the count
        // of knots at or before `unix_s`, minus one, clamped to the last
        // interval so an exact final knot still evaluates.
        let mut index = self
            .knots
            .partition_point(|knot| knot.unix_s <= unix_s)
            .saturating_sub(1);
        if index + 1 >= self.knots.len() {
            index = self.knots.len() - 2;
        }
        let knot0 = self.knots.get(index)?;
        let knot1 = self.knots.get(index + 1)?;
        let span_s = knot1.unix_s - knot0.unix_s;
        if span_s <= 0.0 {
            return None;
        }
        let s = (unix_s - knot0.unix_s) / span_s;
        Some((
            index,
            hermite_position(knot0, knot1, s),
            hermite_velocity(knot0, knot1, s, span_s),
        ))
    }

    /// Fill knot velocities with centred finite differences.
    ///
    /// Interior knots use `(p[i+1] - p[i-1]) / (t[i+1] - t[i-1])`; the ends
    /// fall back to the one-sided secant. Positions are already validated, so
    /// every span is positive.
    fn fill_velocities(&mut self) -> Result<(), AdsbError> {
        let mut velocities = Vec::with_capacity(self.knots.len());
        let knots = &self.knots;

        // First knot: forward secant.
        if let (Some(first), Some(second)) = (knots.first(), knots.get(1)) {
            velocities.push(enu_div(
                enu_sub(second.position, first.position),
                second.unix_s - first.unix_s,
            ));
        }
        // Interior knots: centred difference over the neighbouring pair.
        for window in knots.windows(3) {
            let &[before, _, after] = window else {
                continue;
            };
            velocities.push(enu_div(
                enu_sub(after.position, before.position),
                after.unix_s - before.unix_s,
            ));
        }
        // Last knot: backward secant.
        if let (Some(last), Some(previous)) = (knots.last(), knots.get(knots.len().wrapping_sub(2)))
        {
            velocities.push(enu_div(
                enu_sub(last.position, previous.position),
                last.unix_s - previous.unix_s,
            ));
        }

        if velocities.len() != knots.len() {
            return Err(AdsbError::InvalidTrack(
                "could not difference every knot".to_owned(),
            ));
        }
        for (knot, velocity) in self.knots.iter_mut().zip(velocities) {
            knot.velocity = velocity;
        }
        Ok(())
    }
}

/// Interpolate a track at a UTC time.
///
/// Cubic Hermite in the track's local ENU frame (SDD section 4); see
/// [`TrackInterpolator`] for the scheme and its limits. Returns `None` when
/// the time is outside the track or the track cannot be interpolated.
pub fn interpolate(track: &FlightTrack, at: UtcMillis) -> Option<TrackSample> {
    // Interpolation is best-effort: a track that fails validation or lacks
    // two strictly increasing samples yields no sample.
    TrackInterpolator::new(track).ok()?.sample_at(at)
}

/// Evaluate the cubic position at fraction `s` of the interval.
fn hermite_position(knot0: &Knot, knot1: &Knot, s: f64) -> Enu {
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
    let h10 = s3 - 2.0 * s2 + s;
    let h01 = -2.0 * s3 + 3.0 * s2;
    let h11 = s3 - s2;
    let span_s = knot1.unix_s - knot0.unix_s;
    enu_sum(
        enu_sum(
            enu_scale(knot0.position, h00),
            enu_scale(knot1.position, h01),
        ),
        enu_sum(
            enu_scale(knot0.velocity, h10 * span_s),
            enu_scale(knot1.velocity, h11 * span_s),
        ),
    )
}

/// Evaluate the cubic velocity at fraction `s` of the interval.
fn hermite_velocity(knot0: &Knot, knot1: &Knot, s: f64, span_s: f64) -> Enu {
    let s2 = s * s;
    let dh00 = 6.0 * s2 - 6.0 * s;
    let dh10 = 3.0 * s2 - 4.0 * s + 1.0;
    let dh01 = -6.0 * s2 + 6.0 * s;
    let dh11 = 3.0 * s2 - 2.0 * s;
    enu_div(
        enu_sum(
            enu_sum(
                enu_scale(knot0.position, dh00),
                enu_scale(knot1.position, dh01),
            ),
            enu_sum(
                enu_scale(knot0.velocity, dh10 * span_s),
                enu_scale(knot1.velocity, dh11 * span_s),
            ),
        ),
        span_s,
    )
}

fn unix_seconds(utc: UtcMillis) -> f64 {
    utc.0 as f64 / 1_000.0
}

fn fraction_between(start: UtcMillis, end: UtcMillis, at: UtcMillis) -> Option<f64> {
    let span_ms = end.0 - start.0;
    if span_ms <= 0 {
        return None;
    }
    Some((at.0 - start.0) as f64 / span_ms as f64)
}

fn lerp(start: f64, end: f64, fraction: f64) -> f64 {
    start + (end - start) * fraction
}

fn lerp_optional(start: Option<f64>, end: Option<f64>, fraction: f64) -> Option<f64> {
    Some(lerp(start?, end?, fraction))
}

fn enu_sub(a: Enu, b: Enu) -> Enu {
    Enu {
        east_m: a.east_m - b.east_m,
        north_m: a.north_m - b.north_m,
        up_m: a.up_m - b.up_m,
    }
}

fn enu_sum(a: Enu, b: Enu) -> Enu {
    Enu {
        east_m: a.east_m + b.east_m,
        north_m: a.north_m + b.north_m,
        up_m: a.up_m + b.up_m,
    }
}

fn enu_scale(vector: Enu, factor: f64) -> Enu {
    Enu {
        east_m: vector.east_m * factor,
        north_m: vector.north_m * factor,
        up_m: vector.up_m * factor,
    }
}

fn enu_div(vector: Enu, divisor: f64) -> Enu {
    enu_scale(vector, 1.0 / divisor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::TrackSourceKind;

    const BASE_MS: i64 = 1_762_473_600_000;

    fn track_with(samples: Vec<(i64, f64, f64, f64)>) -> FlightTrack {
        FlightTrack {
            icao24: "a1b2c3".to_owned(),
            callsign: None,
            type_code: None,
            source: TrackSourceKind::OpenSky,
            samples: samples
                .into_iter()
                .map(|(offset_ms, lat, lon, alt)| TrackSample {
                    utc: UtcMillis(BASE_MS + offset_ms),
                    lat_deg: lat,
                    lon_deg: lon,
                    geometric_altitude_m: alt,
                    barometric_altitude_m: None,
                    ground_speed_m_per_s: None,
                    track_deg: None,
                    vertical_rate_m_per_s: None,
                    latency_s: None,
                    flags: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn hermite_passes_through_every_knot() {
        let track = track_with(vec![
            (0, 52.0, 13.0, 10_000.0),
            (1_000, 52.001, 13.002, 10_100.0),
            (2_000, 52.003, 13.001, 10_050.0),
            (3_000, 52.004, 13.004, 10_200.0),
        ]);
        let interp = TrackInterpolator::new(&track).expect("valid track");
        for (offset_ms, lat, lon, alt) in [
            (0, 52.0, 13.0, 10_000.0),
            (1_000, 52.001, 13.002, 10_100.0),
            (2_000, 52.003, 13.001, 10_050.0),
            (3_000, 52.004, 13.004, 10_200.0),
        ] {
            let sample = interp
                .sample_at(UtcMillis(BASE_MS + offset_ms))
                .expect("knot time");
            assert!(
                (sample.lat_deg - lat).abs() < 1e-9,
                "lat at {offset_ms} ms: {} != {lat}",
                sample.lat_deg
            );
            assert!((sample.lon_deg - lon).abs() < 1e-9);
            assert!((sample.geometric_altitude_m - alt).abs() < 1e-6);
        }
    }

    #[test]
    fn straight_constant_velocity_track_is_exact() {
        // A level, constant-heading leg: the cubic must reproduce the line.
        let track = track_with(vec![
            (0, 52.0, 13.0, 10_000.0),
            (1_000, 52.001, 13.0, 10_000.0),
            (2_000, 52.002, 13.0, 10_000.0),
            (3_000, 52.003, 13.0, 10_000.0),
        ]);
        let interp = TrackInterpolator::new(&track).expect("valid track");
        let at = interp
            .sample_at(UtcMillis(BASE_MS + 1_500))
            .expect("mid-interval");
        assert!((at.lat_deg - 52.0015).abs() < 1e-9, "{}", at.lat_deg);
        assert!(at.lon_deg.abs() < 1e-12 || (at.lon_deg - 13.0).abs() < 1e-9);
        let velocity = interp
            .velocity_at(unix_seconds(UtcMillis(BASE_MS + 1_500)))
            .expect("velocity");
        // ~111.19 m per 0.001 deg latitude at 52 N, over one second.
        assert!(
            (velocity.north_m - 111.19).abs() < 0.5,
            "north velocity {}",
            velocity.north_m
        );
        assert!(velocity.east_m.abs() < 1e-6);
        // Constant ellipsoidal height is not constant ENU up: Earth curvature
        // bends the track in the tangent frame by ~v²·t/R ≈ 3 mm/s here.
        assert!(velocity.up_m.abs() < 0.01, "up {}", velocity.up_m);
    }

    #[test]
    fn interpolate_flags_and_bounds() {
        let track = track_with(vec![
            (0, 52.0, 13.0, 10_000.0),
            (1_000, 52.001, 13.0, 10_000.0),
        ]);
        let inside = interpolate(&track, UtcMillis(BASE_MS + 500)).expect("inside");
        assert_ne!(inside.flags & crate::tracks::FLAG_INTERPOLATED, 0);
        assert!(interpolate(&track, UtcMillis(BASE_MS - 1)).is_none());
        assert!(interpolate(&track, UtcMillis(BASE_MS + 2_000)).is_none());
        let single = FlightTrack {
            samples: vec![track.samples[0]],
            ..track.clone()
        };
        assert!(interpolate(&single, UtcMillis(BASE_MS)).is_none());
    }

    #[test]
    fn interpolator_rejects_duplicate_times() {
        let track = track_with(vec![(0, 52.0, 13.0, 10_000.0), (0, 52.001, 13.0, 10_000.0)]);
        assert!(matches!(
            TrackInterpolator::new(&track),
            Err(AdsbError::InvalidTrack(_))
        ));
    }
}
