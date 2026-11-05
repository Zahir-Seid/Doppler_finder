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

    /// Convert an ECEF position back to geodetic latitude, longitude and
    /// ellipsoidal height (FR-DA-02).
    ///
    /// Uses Bowring's closed-form estimate refined by one Newton step on the
    /// ellipsoid equation, which converges to well under a millimetre for any
    /// point outside the centre of the Earth. The height comes from whichever
    /// of `p / cos(lat)` and `z / sin(lat)` is numerically stable at that
    /// latitude, so the result does not lose digits close to the poles.
    pub fn from_ecef(&self, ecef: Ecef) -> Result<(LatLon, f64), GeodesyError> {
        if !ecef.is_finite() {
            return Err(GeodesyError::NotFinite);
        }

        let x_m = ecef.x_m;
        let y_m = ecef.y_m;
        let z_m = ecef.z_m;
        let semi_major = self.semi_major_axis_m;
        let semi_minor = self.semi_minor_axis_m();
        let ecc_sq = self.eccentricity_squared();

        let dist_from_axis = x_m.hypot(y_m);
        // At the poles the longitude is undefined; pick 0 and recover latitude
        // and height directly.
        if dist_from_axis < f64::EPSILON * semi_major {
            let lat_deg = if z_m >= 0.0 { 90.0 } else { -90.0 };
            return Ok((LatLon::new(lat_deg, 0.0), z_m.abs() - semi_minor));
        }

        let lon = y_m.atan2(x_m);

        // Bowring's initial parametric latitude.
        let second_ecc_sq = ecc_sq / (1.0 - ecc_sq);
        let theta = (z_m * semi_major).atan2(dist_from_axis * semi_minor);
        let sin_theta = theta.sin();
        let cos_theta = theta.cos();
        let lat = (z_m + second_ecc_sq * semi_minor * sin_theta.powi(3))
            .atan2(dist_from_axis - ecc_sq * semi_major * cos_theta.powi(3));

        // One Newton refinement on the parametric equation makes the result
        // accurate to floating-point noise.
        let sin_lat = lat.sin();
        let prime_vertical_radius = semi_major / (1.0 - ecc_sq * sin_lat * sin_lat).sqrt();
        let lat = (z_m + ecc_sq * prime_vertical_radius * sin_lat).atan2(dist_from_axis);

        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let prime_vertical_radius = semi_major / (1.0 - ecc_sq * sin_lat * sin_lat).sqrt();
        // Near the poles `cos(lat)` vanishes, so use the z equation there.
        let height_m = if cos_lat.abs() > sin_lat.abs() {
            dist_from_axis / cos_lat - prime_vertical_radius
        } else {
            z_m / sin_lat - prime_vertical_radius * (1.0 - ecc_sq)
        };

        Ok((LatLon::new(lat.to_degrees(), lon.to_degrees()), height_m))
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

impl Ecef {
    /// The Earth's centre.
    pub const ORIGIN: Self = Self {
        x_m: 0.0,
        y_m: 0.0,
        z_m: 0.0,
    };

    /// Construct a position from its components in metres.
    pub const fn new(x_m: f64, y_m: f64, z_m: f64) -> Self {
        Self { x_m, y_m, z_m }
    }

    /// Straight-line distance between two positions in metres.
    pub fn distance_m(self, other: Self) -> f64 {
        let dx = self.x_m - other.x_m;
        let dy = self.y_m - other.y_m;
        let dz = self.z_m - other.z_m;
        dx.hypot(dy).hypot(dz)
    }

    /// Euclidean norm of the position vector in metres.
    pub fn norm_m(self) -> f64 {
        self.x_m.hypot(self.y_m).hypot(self.z_m)
    }

    /// Check that every component is finite.
    pub fn is_finite(self) -> bool {
        self.x_m.is_finite() && self.y_m.is_finite() && self.z_m.is_finite()
    }
}

impl std::ops::Add for Ecef {
    type Output = Self;

    /// Component-wise addition, used for propagation along a track.
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x_m: self.x_m + rhs.x_m,
            y_m: self.y_m + rhs.y_m,
            z_m: self.z_m + rhs.z_m,
        }
    }
}

impl std::ops::Sub for Ecef {
    type Output = Self;

    /// Component-wise subtraction.
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x_m: self.x_m - rhs.x_m,
            y_m: self.y_m - rhs.y_m,
            z_m: self.z_m - rhs.z_m,
        }
    }
}

impl std::ops::Mul<f64> for Ecef {
    type Output = Self;

    /// Scalar multiplication, used to scale a direction into a displacement.
    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            x_m: self.x_m * rhs,
            y_m: self.y_m * rhs,
            z_m: self.z_m * rhs,
        }
    }
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

/// Round-trip tests for FR-DA-02: point conversions must not move a position.
#[cfg(test)]
mod tests {
    use super::*;

    /// Maximum permitted round-trip error, 0.1 mm, well inside the
    /// sub-millimetre bound the geodesy phase promises.
    const TOLERANCE_M: f64 = 1e-4;

    fn assert_round_trip(lat_deg: f64, lon_deg: f64, height_m: f64) {
        let geodetic = LatLon::new(lat_deg, lon_deg);
        let ecef = Ellipsoid::WGS84
            .to_ecef(geodetic, height_m)
            .expect("to_ecef");
        let (back, back_height) = Ellipsoid::WGS84.from_ecef(ecef).expect("from_ecef");

        // Position error, measured in ECEF metres, is the quantity that
        // matters for slant distances.
        let recovered = Ellipsoid::WGS84
            .to_ecef(back, back_height)
            .expect("to_ecef");
        assert!(
            recovered.distance_m(ecef) < TOLERANCE_M,
            "({lat_deg}, {lon_deg}, {height_m}) round-tripped to \
             ({}, {}, {back_height}) with {:e} m error",
            back.lat_deg.0,
            back.lon_deg.0,
            recovered.distance_m(ecef),
        );
        assert!(
            (back_height - height_m).abs() < TOLERANCE_M,
            "height {height_m} round-tripped to {back_height}",
        );
    }

    #[test]
    fn round_trip_holds_on_a_global_grid() {
        for lat in (-90..=90).step_by(15) {
            for lon in (-180..=180).step_by(15) {
                for height in [-1_000.0, 0.0, 1_000.0, 10_000.0] {
                    assert_round_trip(f64::from(lat), f64::from(lon), height);
                }
            }
        }
    }

    #[test]
    fn round_trip_holds_at_the_equator_and_poles() {
        assert_round_trip(0.0, 0.0, 0.0);
        assert_round_trip(0.0, 180.0, 0.0);
        assert_round_trip(90.0, 0.0, 0.0);
        assert_round_trip(-90.0, 0.0, 0.0);
        assert_round_trip(89.999_999, 123.4, 500.0);
    }

    #[test]
    fn round_trip_holds_close_to_the_poles() {
        // `p / cos(lat)` loses digits here, so the z branch has to carry the
        // height. 1e-8 degrees is about 1 mm from the pole.
        assert_round_trip(89.999_999_99, 45.0, 0.0);
        assert_round_trip(89.999_999_99, 45.0, 10_000.0);
        assert_round_trip(-89.999_999_99, -120.0, 500.0);
    }

    #[test]
    fn round_trip_handles_the_antimeridian() {
        assert_round_trip(10.0, -180.0, 0.0);
        assert_round_trip(10.0, 180.0, 0.0);
    }

    #[test]
    fn ecef_origin_distance_and_norm() {
        let a = Ecef::new(1.0, 2.0, 2.0);
        assert!(a.is_finite());
        assert!((a.norm_m() - 3.0).abs() < 1e-12);
        assert!((a.distance_m(Ecef::ORIGIN) - 3.0).abs() < 1e-12);
        assert_eq!(a + Ecef::ORIGIN, a);
        assert_eq!(a - a, Ecef::ORIGIN);
        assert_eq!(a * 2.0, Ecef::new(2.0, 4.0, 4.0));
    }

    #[test]
    fn rejects_non_finite_input() {
        let bad = Ecef::new(f64::NAN, 0.0, 0.0);
        assert!(matches!(
            Ellipsoid::WGS84.from_ecef(bad),
            Err(GeodesyError::NotFinite)
        ));
    }
}
