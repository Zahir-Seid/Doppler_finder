//! Measured atmosphere adapter (FR-DA-03, SDD T-2, Q-10).
//!
//! Weather archives report temperature, pressure, humidity and a wind vector
//! on a time grid. This module ingests a recorded payload, averages the
//! conditions over a window, and projects the mean wind onto each
//! source-observer path. When no usable sample exists the caller gets
//! [`Atmosphere::standard`], whose [`AtmosphereSource::StandardFallback`]
//! tag is what the run manifest records — the fallback is always explicit,
//! never silent.
//!
//! The payload format is the normalized ingest shape: a JSON object with a
//! `samples` array of `{unix_s, temperature_k, pressure_pa,
//! relative_humidity, wind_east_m_per_s, wind_north_m_per_s}`. A fetch
//! adapter for a live archive (Open-Meteo or similar) is separate work; the
//! parser is transport-free so tests replay recorded fixtures only.
//!
//! Wind sign convention (Q-10, now fixed here): vector components are the
//! direction the air moves **toward**, east and north positive. The
//! along-path component returned by [`Wind::towards`] is positive when the
//! wind blows from the source toward the observer, shortening propagation —
//! the same convention [`Atmosphere::effective_speed_m_per_s`] documents.

use core_types::LatLon;
use geodesy::LocalFrame;
use serde::Deserialize;

use crate::atmosphere::Atmosphere;
use crate::error::AcousticsError;

/// One weather observation on the archive's time grid.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct WeatherSample {
    /// Observation time, seconds since the Unix epoch.
    pub unix_s: f64,
    /// Air temperature, kelvin.
    pub temperature_k: f64,
    /// Static pressure, pascals.
    pub pressure_pa: f64,
    /// Relative humidity as a fraction in `[0, 1]`.
    pub relative_humidity: f64,
    /// Eastward wind component, metres per second; positive east.
    pub wind_east_m_per_s: f64,
    /// Northward wind component, metres per second; positive north.
    pub wind_north_m_per_s: f64,
}

/// Mean conditions over a window, ready for [`Atmosphere::measured`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conditions {
    /// Mean air temperature, kelvin.
    pub temperature_k: f64,
    /// Mean static pressure, pascals.
    pub pressure_pa: f64,
    /// Mean relative humidity as a fraction in `[0, 1]`.
    pub relative_humidity: f64,
    /// Mean wind vector over the window.
    pub wind: Wind,
}

/// A horizontal wind vector in metres per second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Wind {
    /// Eastward component, metres per second.
    pub east_m_per_s: f64,
    /// Northward component, metres per second.
    pub north_m_per_s: f64,
}

impl Wind {
    /// Wind component along the path from `source` to `observer`, metres per
    /// second. Positive when the wind blows from the source toward the
    /// observer (Q-10), which shortens propagation time. Only the horizontal
    /// projection is used; vertical wind is not reported by the archives.
    ///
    /// Returns [`AcousticsError::InvalidQuery`] when the two positions are
    /// the same point, leaving no path direction.
    pub fn towards(&self, source: LatLon, observer: LatLon) -> Result<f64, AcousticsError> {
        let frame = LocalFrame::wgs84(source).map_err(|_| AcousticsError::NotFinite)?;
        let delta = frame
            .to_enu(observer, 0.0)
            .map_err(|_| AcousticsError::NotFinite)?;
        let distance_m = delta.horizontal_distance_m();
        if distance_m <= 0.0 || !distance_m.is_finite() {
            return Err(AcousticsError::InvalidQuery(
                "source and observer coincide".to_owned(),
            ));
        }
        Ok((self.east_m_per_s * delta.east_m + self.north_m_per_s * delta.north_m) / distance_m)
    }
}

/// Parse the normalized weather payload into samples sorted by time.
///
/// # Errors
///
/// Returns [`AcousticsError::Parse`] for invalid JSON, a missing `samples`
/// array, a malformed sample, or a non-finite field.
pub fn parse_weather(payload: &str) -> Result<Vec<WeatherSample>, AcousticsError> {
    #[derive(Deserialize)]
    struct Payload {
        samples: Vec<WeatherSample>,
    }
    let payload: Payload = serde_json::from_str(payload)
        .map_err(|error| AcousticsError::Parse(format!("invalid weather JSON: {error}")))?;
    for sample in &payload.samples {
        for (name, value) in [
            ("unix_s", sample.unix_s),
            ("temperature_k", sample.temperature_k),
            ("pressure_pa", sample.pressure_pa),
            ("relative_humidity", sample.relative_humidity),
            ("wind_east_m_per_s", sample.wind_east_m_per_s),
            ("wind_north_m_per_s", sample.wind_north_m_per_s),
        ] {
            if !value.is_finite() {
                return Err(AcousticsError::Parse(format!(
                    "weather field `{name}` must be finite"
                )));
            }
        }
    }
    let mut samples = payload.samples;
    samples.sort_by(|a, b| a.unix_s.total_cmp(&b.unix_s));
    Ok(samples)
}

/// Average the samples whose `unix_s` lies in `[from_s, to_s]`, inclusive.
///
/// Returns `None` when no sample falls inside the window. Wind is averaged
/// as a vector, so opposing winds partially cancel rather than averaging
/// their speeds.
pub fn conditions_in_window(
    samples: &[WeatherSample],
    from_s: f64,
    to_s: f64,
) -> Option<Conditions> {
    if !from_s.is_finite() || !to_s.is_finite() || from_s > to_s {
        return None;
    }
    let mut count = 0usize;
    let mut temperature_k = 0.0;
    let mut pressure_pa = 0.0;
    let mut relative_humidity = 0.0;
    let mut wind_east = 0.0;
    let mut wind_north = 0.0;
    for sample in samples {
        if sample.unix_s < from_s || sample.unix_s > to_s {
            continue;
        }
        count += 1;
        temperature_k += sample.temperature_k;
        pressure_pa += sample.pressure_pa;
        relative_humidity += sample.relative_humidity;
        wind_east += sample.wind_east_m_per_s;
        wind_north += sample.wind_north_m_per_s;
    }
    if count == 0 {
        return None;
    }
    let n = count as f64;
    Some(Conditions {
        temperature_k: temperature_k / n,
        pressure_pa: pressure_pa / n,
        relative_humidity: relative_humidity / n,
        wind: Wind {
            east_m_per_s: wind_east / n,
            north_m_per_s: wind_north / n,
        },
    })
}

/// Build the [`Atmosphere`] for a window: the measured mean when samples
/// exist, otherwise the flagged [`Atmosphere::standard`] fallback.
///
/// The fallback is the explicit T-2 path: callers read
/// [`Atmosphere::is_standard_fallback`] and record
/// [`AtmosphereSource::StandardFallback`] in the manifest.
///
/// Returns [`AcousticsError`] when the measured mean fails
/// [`Atmosphere::measured`] validation.
pub fn atmosphere_for_window(
    samples: &[WeatherSample],
    from_s: f64,
    to_s: f64,
) -> Result<Atmosphere, AcousticsError> {
    match conditions_in_window(samples, from_s, to_s) {
        Some(conditions) => Atmosphere::measured(
            conditions.temperature_k,
            conditions.pressure_pa,
            conditions.relative_humidity,
        ),
        None => Ok(Atmosphere::standard()),
    }
}
