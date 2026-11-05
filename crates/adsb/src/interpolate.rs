//! Linear interpolation between track samples.

use core_types::UtcMillis;

use crate::tracks::{FlightTrack, TrackSample};

/// Interpolate a track at a UTC time.
///
/// Returns `None` when the time is outside the track or the track has no
/// usable bracketing pair. Coordinates and altitude are interpolated linearly;
/// this is the first-order scheme used before the light-time solver refines it.
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
            ground_speed_m_per_s: first.ground_speed_m_per_s,
            track_deg: first.track_deg,
        })
    })
}

fn lerp(start: f64, end: f64, fraction: f64) -> f64 {
    start + (end - start) * fraction
}
