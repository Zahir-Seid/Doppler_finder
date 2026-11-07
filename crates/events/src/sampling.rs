//! Sample the whitened z-map along an emission-time model curve.
//!
//! The direct estimator (FR-DM-05) and the quality gate (FR-DM-06) must agree
//! on what "power along the curve" means, so both call this module instead of
//! carrying their own interpolation. A frequency between two bins is sampled
//! by linear interpolation of the z values and, where power is needed, the
//! result is squared.

use dsp::ZMap;

use crate::fit::{Parameters, predicted_hz_at};

/// Linearly interpolated z value at one frame and frequency.
///
/// Returns `None` when the frequency is non-finite, non-positive or outside the
/// map, so a curve that leaves the measured band is not silently clamped to an
/// edge bin.
pub(crate) fn interpolated_z(map: &ZMap, frame: usize, frequency_hz: f64) -> Option<f64> {
    if !frequency_hz.is_finite() || frequency_hz <= 0.0 {
        return None;
    }
    let bins = map.bin_count();
    let width_hz = map.config().bin_width_hz();
    if !width_hz.is_finite() || width_hz <= 0.0 {
        return None;
    }
    let coordinate = frequency_hz / width_hz;
    let last_index = bins.checked_sub(1)?;
    let last = last_index as f64;
    if !coordinate.is_finite() || coordinate < 0.0 || coordinate > last {
        return None;
    }
    let lower = coordinate.floor();
    let fraction = coordinate - lower;
    let lower_index = lower as usize;
    let upper_index = lower_index.saturating_add(1).min(last_index);
    let lower_z = f64::from(map.value(frame, lower_index)?);
    let upper_z = f64::from(map.value(frame, upper_index)?);
    Some(lower_z + (upper_z - lower_z) * fraction)
}

/// z values along each requested harmonic, in frame order.
///
/// Frames whose model frequency falls outside the map are skipped for that
/// harmonic. An empty inner vector means the whole harmonic was out of band.
pub(crate) fn curve_z(
    map: &ZMap,
    parameters: &Parameters,
    harmonics: &[u32],
    sound_speed_m_per_s: f64,
) -> Vec<Vec<f64>> {
    let mut curves = Vec::with_capacity(harmonics.len());
    for &harmonic in harmonics {
        let mut values = Vec::with_capacity(map.frame_count());
        for (frame, &time_s) in map.frame_times_s().iter().enumerate() {
            let Some(frequency_hz) =
                predicted_hz_at(parameters, time_s, harmonic, sound_speed_m_per_s)
            else {
                continue;
            };
            if let Some(z) = interpolated_z(map, frame, frequency_hz) {
                values.push(z);
            }
        }
        curves.push(values);
    }
    curves
}
