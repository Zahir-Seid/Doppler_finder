//! Closest-approach prediction for one track and one observer (SDD 6.1).
//!
//! For a hypothesis (observer `x`, flight `j`) the prediction stage needs the
//! instant of minimum 3-D slant range, the range itself, the aircraft's speed
//! there, and the time the sound emitted at that instant is heard. The
//! position comes from the Hermite interpolant of [`crate::interpolate`]; the
//! heard time applies the light-time relation `t_r = t_e + range / c` exactly,
//! which needs no iteration because the emission time is already known — the
//! bracketed solver in `acoustics` is for the inverse direction used when
//! matching events.
//!
//! The minimum is found by a golden-section search of the squared slant range
//! on every interpolation interval, so curved legs are handled interval by
//! interval rather than assuming a straight pass.

use core_types::{LatLon, UtcMillis};
use geodesy::Ellipsoid;

use crate::error::AdsbError;
use crate::interpolate::TrackInterpolator;
use crate::tracks::FlightTrack;

/// Golden-section iterations per interval. Sixty-four halvings of the search
/// window leave far below millimetre position error on any propeller leg.
const GOLDEN_ITERATIONS: usize = 64;

/// Inverse of the golden ratio conjugate, for the section search.
const INV_PHI: f64 = 0.618_033_988_749_894_9;

/// The predicted closest approach of a track to an observer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosestApproach {
    /// Emission time of minimum slant range, in UTC milliseconds.
    pub emission_utc: UtcMillis,
    /// Time the sound emitted at closest approach is heard, in UTC
    /// milliseconds: `emission_utc + slant_range_m / sound_speed`.
    pub heard_utc: UtcMillis,
    /// Minimum 3-D slant range in metres.
    pub slant_range_m: f64,
    /// Horizontal speed over ground at closest approach, in metres per second.
    pub speed_m_per_s: f64,
}

/// Find the closest approach of `track` to an observer.
///
/// `observer` is the geodetic position and `observer_height_m` the
/// ellipsoidal height of the listener (terrain plus ear height plus geoid
/// undulation, as `geodesy::observer_height_m` returns). `sound_speed_m_per_s`
/// comes from the atmosphere model, never a literal.
///
/// Returns `Ok(None)` when the track has fewer than two samples and cannot be
/// interpolated. Returns [`AdsbError::InvalidQuery`] for a non-finite or
/// non-positive sound speed, a bad observer position or height, and
/// [`AdsbError::InvalidTrack`] when the track itself is unusable.
///
/// # Errors
///
/// See the return documentation above.
pub fn closest_approach(
    track: &FlightTrack,
    observer: LatLon,
    observer_height_m: f64,
    sound_speed_m_per_s: f64,
) -> Result<Option<ClosestApproach>, AdsbError> {
    if !sound_speed_m_per_s.is_finite() || sound_speed_m_per_s <= 0.0 {
        return Err(AdsbError::InvalidQuery(
            "sound speed must be finite and positive".to_owned(),
        ));
    }
    if !observer_height_m.is_finite() {
        return Err(AdsbError::InvalidQuery(
            "observer height must be finite".to_owned(),
        ));
    }
    observer
        .validate()
        .map_err(|error| AdsbError::InvalidQuery(format!("observer: {error}")))?;
    track.validate()?;
    if track.samples.len() < 2 {
        return Ok(None);
    }

    let interpolator = TrackInterpolator::new(track)?;
    let observer_ecef = Ellipsoid::WGS84.to_ecef(observer, observer_height_m)?;
    let range_sq = |unix_s: f64| -> Option<f64> {
        let position = interpolator.position_ecef(unix_s)?;
        let range_m = position.distance_m(observer_ecef);
        Some(range_m * range_m)
    };

    let mut best: Option<(f64, f64)> = None;
    for (lo, hi) in interpolator.intervals() {
        let (time_s, value) = golden_min(range_sq, lo, hi);
        if best.is_none_or(|(_, best_value)| value < best_value) {
            best = Some((time_s, value));
        }
    }
    // A two-knot track still has one interval, but guard the degenerate case
    // where every evaluation failed.
    let (emission_s, range_sq_min) = best.ok_or(AdsbError::InvalidTrack(
        "closest approach found no evaluable position".to_owned(),
    ))?;
    let slant_range_m = range_sq_min.sqrt();
    let velocity = interpolator
        .velocity_at(emission_s)
        .ok_or(AdsbError::InvalidTrack(
            "closest approach found no evaluable velocity".to_owned(),
        ))?;
    let speed_m_per_s = velocity.east_m.hypot(velocity.north_m);

    let emission_utc = unix_to_utc_millis(emission_s);
    let heard_s = emission_s + slant_range_m / sound_speed_m_per_s;
    Ok(Some(ClosestApproach {
        emission_utc,
        heard_utc: unix_to_utc_millis(heard_s),
        slant_range_m,
        speed_m_per_s,
    }))
}

/// Minimize `f` on `[lo, hi]` by golden-section search.
///
/// `f` returns `None` outside the interpolant's span; evaluations inside the
/// interval always succeed, so a `None` is treated as `+inf` and the search
/// moves away from it.
fn golden_min(f: impl Fn(f64) -> Option<f64>, lo: f64, hi: f64) -> (f64, f64) {
    let mut left = lo;
    let mut right = hi;
    let mut inner_left = right - INV_PHI * (right - left);
    let mut inner_right = left + INV_PHI * (right - left);
    let mut value_left = f(inner_left).unwrap_or(f64::INFINITY);
    let mut value_right = f(inner_right).unwrap_or(f64::INFINITY);
    for _ in 0..GOLDEN_ITERATIONS {
        if value_left < value_right {
            right = inner_right;
            inner_right = inner_left;
            value_right = value_left;
            inner_left = right - INV_PHI * (right - left);
            value_left = f(inner_left).unwrap_or(f64::INFINITY);
        } else {
            left = inner_left;
            inner_left = inner_right;
            value_left = value_right;
            inner_right = left + INV_PHI * (right - left);
            value_right = f(inner_right).unwrap_or(f64::INFINITY);
        }
    }
    let midpoint = f64::midpoint(left, right);
    let value = f(midpoint).unwrap_or(f64::INFINITY);
    (midpoint, value)
}

fn unix_to_utc_millis(unix_s: f64) -> UtcMillis {
    UtcMillis((unix_s * 1_000.0).round() as i64)
}
