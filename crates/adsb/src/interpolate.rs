//! Linear interpolation between track samples.

use core_types::UtcMillis;

use crate::tracks::{FlightTrack, TrackSample};

/// Interpolate a track at a UTC time.
///
/// Returns `None` when the time is outside the track or the track has no
/// usable bracketing pair. Coordinates and altitude are interpolated linearly;
/// this is the first-order scheme used before the light-time solver refines it.
/// Optional barometric altitude and vertical rate are interpolated only when
/// both bracketing samples carry them; ground speed and track angle are copied
/// from the earlier sample. The result is flagged
/// [`crate::tracks::FLAG_INTERPOLATED`].
pub fn interpolate(track: &FlightTrack, at: UtcMillis) -> Option<TrackSample> {
    track.samples.windows(2).find_map(|pair| {
        let first = pair.first()?;
        let second = pair.get(1)?;
        if at.0 < first.utc.0 || at.0 > second.utc.0 {
            return None;
        }
        if second.utc.0 == first.utc.0 {
            return None;
        }
        let span_ms = (second.utc.0 - first.utc.0) as f64;
        let offset_ms = (at.0 - first.utc.0) as f64;
        let fraction = offset_ms / span_ms;
        Some(TrackSample {
            utc: at,
            lat_deg: lerp(first.lat_deg, second.lat_deg, fraction),
            lon_deg: lerp(first.lon_deg, second.lon_deg, fraction),
            geometric_altitude_m: lerp(
                first.geometric_altitude_m,
                second.geometric_altitude_m,
                fraction,
            ),
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
            flags: first.flags | second.flags | crate::tracks::FLAG_INTERPOLATED,
        })
    })
}

fn lerp(start: f64, end: f64, fraction: f64) -> f64 {
    start + (end - start) * fraction
}

fn lerp_optional(start: Option<f64>, end: Option<f64>, fraction: f64) -> Option<f64> {
    Some(lerp(start?, end?, fraction))
}
