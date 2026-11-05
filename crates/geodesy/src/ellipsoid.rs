//! WGS84 ellipsoid and Earth-centred, Earth-fixed coordinates.

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use crate::error::GeodesyError;

/// A geodetic reference ellipsoid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ellipsoid {
    /// Semi-major axis in metres.
    pub semi_major_axis_m: f64,
    /// Flattening, dimensionless.
    pub flattening: f64,
}

impl Ellipsoid {
    /// The WGS84 ellipsoid (SDD section 4).
    pub const WGS84: Self = Self {
        semi_major_axis_m: 6_378_137.0,
        flattening: 1.0 / 298.257_223_563,
    };

    /// Semi-minor axis in metres.
    pub fn semi_minor_axis_m(&self) -> f64 {
        self.semi_major_axis_m * (1.0 - self.flattening)
    }

    /// First eccentricity squared.
    pub fn eccentricity_squared(&self) -> f64 {
        self.flattening * (2.0 - self.flattening)
    }

    /// Convert a geodetic position and ellipsoidal height to ECEF.
    ///
    /// Returns [`GeodesyError`] when the latitude or longitude is outside its
    /// valid range.
    pub fn to_ecef(&self, position: LatLon, height_m: f64) -> Result<Ecef, GeodesyError> {
        validate(position)?;
        if !height_m.is_finite() {
            return Err(GeodesyError::NotFinite);
        }

        let lat = position.lat_deg.0.to_radians();
        let lon = position.lon_deg.0.to_radians();
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();
        let eccentricity_squared = self.eccentricity_squared();
        let prime_vertical_radius =
            self.semi_major_axis_m / (1.0 - eccentricity_squared * sin_lat * sin_lat).sqrt();

        Ok(Ecef {
            x_m: (prime_vertical_radius + height_m) * cos_lat * cos_lon,
            y_m: (prime_vertical_radius + height_m) * cos_lat * sin_lon,
            z_m: (prime_vertical_radius * (1.0 - eccentricity_squared) + height_m) * sin_lat,
        })
    }
}

/// Earth-centred, Earth-fixed position in metres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ecef {
    /// X axis in metres, through latitude 0 and longitude 0.
    pub x_m: f64,
    /// Y axis in metres, through latitude 0 and longitude 90 degrees east.
    pub y_m: f64,
    /// Z axis in metres, through the north pole.
    pub z_m: f64,
}

/// Validate a geodetic position.
pub fn validate(position: LatLon) -> Result<(), GeodesyError> {
    let lat = position.lat_deg.0;
    let lon = position.lon_deg.0;
    if !lat.is_finite() || !lon.is_finite() {
        return Err(GeodesyError::NotFinite);
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(GeodesyError::LatitudeOutOfRange(lat));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(GeodesyError::LongitudeOutOfRange(lon));
    }
    Ok(())
}
