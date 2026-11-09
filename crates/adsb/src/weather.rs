//! Open-Meteo weather archive client (FR-DA-03).
//!
//! Fetches hourly surface conditions for one location and time window and
//! emits the normalized payload that [`acoustics::weather::parse_weather`]
//! ingests: `{"samples": [{unix_s, temperature_k, pressure_pa,
//! relative_humidity, wind_east_m_per_s, wind_north_m_per_s}]}`. This crate
//! does not depend on `acoustics`; the payload string is the seam, so the
//! fetch adapter stays inside the I/O boundary SDD section 3 assigns to
//! `adsb`.
//!
//! Two Open-Meteo endpoints cover the archive:
//!
//! - [`ARCHIVE_API_URL`]: ERA5 reanalysis archive, roughly 1940 to five days
//!   ago — the right source for recordings made in the past.
//! - [`HIST_FORECAST_API_URL`]: operational forecast archive, roughly the
//!   last two years including recent days the reanalysis archive has not
//!   published yet.
//!
//! Every request goes through [`HttpPolicy::fetch`], so the host must appear
//! on the caller's allow-list ([`OPEN_METEO_HOSTS`]). Tests replay recorded
//! responses through a scripted [`HttpTransport`]; no test opens a socket.
//!
//! Response handling follows the reference implementation's conventions
//! (`fdop-data/src/weather.rs`): wind speed is requested in m/s and wind
//! direction is the meteorological *from* direction in degrees, converted to
//! the toward-components `acoustics::weather::Wind` documents (Q-10). Rows
//! with a missing value are skipped rather than fabricated; structure-level
//! problems are [`AdsbError::Parse`].

use crate::error::AdsbError;
use crate::http::{HttpPolicy, HttpRequest, HttpTransport};
use crate::track_store::civil_from_days;

/// Open-Meteo ERA5 archive endpoint (`archive-api.open-meteo.com`).
pub const ARCHIVE_API_URL: &str = "https://archive-api.open-meteo.com/v1/archive";

/// Open-Meteo historical-forecast endpoint (`historical-forecast-api.open-meteo.com`).
pub const HIST_FORECAST_API_URL: &str =
    "https://historical-forecast-api.open-meteo.com/v1/forecast";

/// Open-Meteo forecast endpoint (`api.open-meteo.com`).
pub const FORECAST_API_URL: &str = "https://api.open-meteo.com/v1/forecast";

/// Hosts the weather client may contact; add them to the [`HttpPolicy`]
/// allow-list when fetching weather.
pub const OPEN_METEO_HOSTS: [&str; 3] = [
    "archive-api.open-meteo.com",
    "historical-forecast-api.open-meteo.com",
    "api.open-meteo.com",
];

const HOURLY_VARIABLES: &str = "temperature_2m,relative_humidity_2m,\
surface_pressure,wind_speed_10m,wind_direction_10m";

/// Which Open-Meteo product to query. [`WeatherArchive::Archive`] is the
/// default for recordings; [`WeatherArchive::HistoricalForecast`] covers
/// recent days the archive has not published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherArchive {
    /// ERA5 reanalysis archive (`ARCHIVE_API_URL`).
    Archive,
    /// Operational forecast archive (`HIST_FORECAST_API_URL`).
    HistoricalForecast,
}

impl WeatherArchive {
    /// Endpoint base URL for this product.
    pub fn base_url(self) -> &'static str {
        match self {
            Self::Archive => ARCHIVE_API_URL,
            Self::HistoricalForecast => HIST_FORECAST_API_URL,
        }
    }
}

/// Build the Open-Meteo request URL for one location and UTC day window.
///
/// `from_s`/`to_s` are Unix seconds; the API works on whole UTC dates, so the
/// request covers the dates the window touches and the returned samples may
/// fall outside it — `acoustics::weather::conditions_in_window` does the
/// exact windowing downstream.
///
/// # Errors
///
/// Returns [`AdsbError::InvalidQuery`] when `from_s > to_s` or either bound is
/// not finite.
pub fn open_meteo_url(
    archive: WeatherArchive,
    latitude_deg: f64,
    longitude_deg: f64,
    from_s: f64,
    to_s: f64,
) -> Result<String, AdsbError> {
    if !from_s.is_finite() || !to_s.is_finite() || from_s > to_s {
        return Err(AdsbError::InvalidQuery(
            "weather window must be finite and ordered".to_owned(),
        ));
    }
    if !(-90.0..=90.0).contains(&latitude_deg) || !(-180.0..=180.0).contains(&longitude_deg) {
        return Err(AdsbError::InvalidQuery(
            "weather position outside WGS84 bounds".to_owned(),
        ));
    }
    let (start, end) = (date_from_unix_s(from_s), date_from_unix_s(to_s));
    Ok(format!(
        "{}?latitude={latitude_deg}&longitude={longitude_deg}\
         &start_date={start}&end_date={end}\
         &hourly={HOURLY_VARIABLES}\
         &wind_speed_unit=ms&timeformat=unixtime&timezone=GMT",
        archive.base_url(),
    ))
}

/// Fetch hourly weather for one location and emit the normalized payload.
///
/// The payload round-trips through `acoustics::weather::parse_weather`;
/// callers hand the string to that parser unchanged.
///
/// # Errors
///
/// Returns the transport/policy errors from [`HttpPolicy::fetch`],
/// [`AdsbError::Parse`] for a malformed Open-Meteo response, or
/// [`AdsbError::InvalidQuery`] for a bad window or position.
pub fn fetch_weather_payload<T: HttpTransport>(
    policy: &HttpPolicy,
    transport: &T,
    archive: WeatherArchive,
    latitude_deg: f64,
    longitude_deg: f64,
    from_s: f64,
    to_s: f64,
) -> Result<String, AdsbError> {
    let url = open_meteo_url(archive, latitude_deg, longitude_deg, from_s, to_s)?;
    let response = policy.fetch(transport, &HttpRequest::new(url, None))?;
    parse_open_meteo_response(response.body())
}

/// Convert an Open-Meteo response body into the normalized payload string.
///
/// Exposed for tests and for callers that cache raw responses themselves.
///
/// # Errors
///
/// Returns [`AdsbError::Parse`] when the body is not JSON, lacks the hourly
/// block, or has a missing time axis.
pub fn parse_open_meteo_response(body: &[u8]) -> Result<String, AdsbError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| AdsbError::Parse(format!("invalid Open-Meteo JSON: {error}")))?;
    // A single-location request returns one object; a comma-separated
    // coordinate list returns an array. This client always asks for one
    // location, but accept the array form and take the first element so a
    // hand-built request cannot silently produce the wrong site's weather.
    let location = match &value {
        serde_json::Value::Array(items) => items.first().unwrap_or(&serde_json::Value::Null),
        other => other,
    };
    let hourly = location
        .get("hourly")
        .ok_or_else(|| AdsbError::Parse("Open-Meteo response has no hourly block".to_owned()))?;
    let times = hourly
        .get("time")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AdsbError::Parse("Open-Meteo response has no time axis".to_owned()))?;

    let mut samples = Vec::with_capacity(times.len());
    for (index, time) in times.iter().enumerate() {
        let Some(unix_s) = time.as_f64() else {
            continue;
        };
        let Some(sample) = hourly_sample(hourly, index, unix_s) else {
            continue;
        };
        samples.push(sample);
    }
    serde_json::to_string(&serde_json::json!({ "samples": samples }))
        .map_err(|error| AdsbError::Parse(format!("could not encode weather payload: {error}")))
}

/// One hourly row -> normalized sample, or `None` when any field is missing.
///
/// Unit conversions: °C → K, hPa → Pa, % → fraction. Wind arrives as speed
/// (m/s, per `wind_speed_unit=ms`) and *from*-direction degrees; the payload
/// wants toward-components, matching `acoustics::weather::Wind` (Q-10).
fn hourly_sample(
    hourly: &serde_json::Value,
    index: usize,
    unix_s: f64,
) -> Option<serde_json::Value> {
    let temperature_c = hourly.get("temperature_2m")?.get(index)?.as_f64()?;
    let humidity_pct = hourly.get("relative_humidity_2m")?.get(index)?.as_f64()?;
    let pressure_hpa = hourly.get("surface_pressure")?.get(index)?.as_f64()?;
    let wind_speed = hourly.get("wind_speed_10m")?.get(index)?.as_f64()?;
    let wind_from_deg = hourly.get("wind_direction_10m")?.get(index)?.as_f64()?;
    if ![
        temperature_c,
        humidity_pct,
        pressure_hpa,
        wind_speed,
        wind_from_deg,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return None;
    }
    let radians = wind_from_deg.to_radians();
    Some(serde_json::json!({
        "unix_s": unix_s,
        "temperature_k": temperature_c + 273.15,
        "pressure_pa": pressure_hpa * 100.0,
        "relative_humidity": humidity_pct / 100.0,
        "wind_east_m_per_s": -wind_speed * radians.sin(),
        "wind_north_m_per_s": -wind_speed * radians.cos(),
    }))
}

/// `YYYY-MM-DD` for a Unix-seconds timestamp.
fn date_from_unix_s(unix_s: f64) -> String {
    let days = (unix_s as i64).div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_covers_window_dates() {
        // 2024-01-15T12:00Z .. 2024-01-16T12:00Z
        let url = open_meteo_url(
            WeatherArchive::Archive,
            52.52,
            13.405,
            1_705_320_000.0,
            1_705_406_400.0,
        )
        .expect("valid window");
        assert!(url.starts_with(ARCHIVE_API_URL));
        assert!(url.contains("start_date=2024-01-15"));
        assert!(url.contains("end_date=2024-01-16"));
        assert!(url.contains("wind_speed_unit=ms"));
        assert!(url.contains("timeformat=unixtime"));
        assert!(url.contains("timezone=GMT"));
    }

    #[test]
    fn url_rejects_bad_window() {
        assert!(open_meteo_url(WeatherArchive::Archive, 0.0, 0.0, 10.0, 5.0).is_err());
        assert!(open_meteo_url(WeatherArchive::Archive, 0.0, 0.0, f64::NAN, 5.0).is_err());
        assert!(open_meteo_url(WeatherArchive::Archive, 91.0, 0.0, 0.0, 5.0).is_err());
    }
}
