//! Physical quantity newtypes.
//!
//! Units are part of the type, never a comment or a bare `f64` (Norms section
//! 5). Degrees are converted to radians at the geodesy boundary; radians are
//! used for every internal angle.
//!
//! Every newtype is `#[repr(transparent)]` over an `f64`, so a slice of one can
//! be reinterpreted where a C ABI or a buffer layout requires it, but the Rust
//! type system keeps the quantities apart. Additive and scalar operations are
//! implemented; mixing two different units requires an explicit method call.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// A frequency in hertz.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Hertz(pub f64);

/// A time interval or duration in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Seconds(pub f64);

/// A length in metres.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Metres(pub f64);

/// A speed in metres per second.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct MetresPerSecond(pub f64);

/// An angle in radians.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Radians(pub f64);

/// A latitude in degrees, positive north.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct LatitudeDegrees(pub f64);

/// A longitude in degrees, positive east.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct LongitudeDegrees(pub f64);

/// A geodetic latitude and longitude pair in degrees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LatLon {
    /// Latitude in degrees, positive north.
    pub lat_deg: LatitudeDegrees,
    /// Longitude in degrees, positive east.
    pub lon_deg: LongitudeDegrees,
}

impl LatLon {
    /// Construct a latitude/longitude pair from degree values.
    ///
    /// Values are not validated here; [`LatLon::validate`] returns
    /// [`CoreError`] for an out-of-range or non-finite pair.
    pub const fn new(lat_deg: f64, lon_deg: f64) -> Self {
        Self {
            lat_deg: LatitudeDegrees(lat_deg),
            lon_deg: LongitudeDegrees(lon_deg),
        }
    }

    /// Check that the pair lies on the ellipsoid.
    pub fn validate(self) -> Result<(), CoreError> {
        let lat = self.lat_deg.0;
        let lon = self.lon_deg.0;
        if !lat.is_finite() || !lon.is_finite() {
            return Err(CoreError::NotFinite);
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(CoreError::LatitudeOutOfRange(lat));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(CoreError::LongitudeOutOfRange(lon));
        }
        Ok(())
    }
}

// ---- Hertz ---------------------------------------------------------------

impl Hertz {
    /// Angular frequency in radians per second.
    pub fn angular(self) -> Radians {
        Radians(self.0 * core::f64::consts::TAU)
    }
}

impl Mul<f64> for Hertz {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        Self(self.0 * rhs)
    }
}

// ---- Seconds -------------------------------------------------------------

impl Seconds {
    /// Milliseconds as an `f64`.
    pub fn as_millis_f64(self) -> f64 {
        self.0 * 1_000.0
    }

    /// Whole milliseconds, rounded toward zero, saturating at the `i64` bounds.
    pub fn as_millis_i64(self) -> i64 {
        let millis = self.0 * 1_000.0;
        if millis.is_nan() { 0 } else { millis as i64 }
    }

    /// The absolute value of the interval.
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// Return `true` when the interval is finite and non-negative.
    pub fn is_non_negative(self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }
}

impl Add for Seconds {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Seconds {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Seconds {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Seconds {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Mul<f64> for Seconds {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<f64> for Seconds {
    type Output = Self;
    fn div(self, rhs: f64) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl Div for Seconds {
    type Output = f64;
    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

// ---- Metres --------------------------------------------------------------

impl Metres {
    /// Kilometres as an `f64`.
    pub fn as_km(self) -> f64 {
        self.0 / 1_000.0
    }

    /// Construct from kilometres.
    pub fn from_km(value: f64) -> Self {
        Self(value * 1_000.0)
    }

    /// Return `true` when the length is finite and non-negative.
    pub fn is_non_negative(self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }
}

impl Add for Metres {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Metres {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Metres {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Mul<f64> for Metres {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<f64> for Metres {
    type Output = Self;
    fn div(self, rhs: f64) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl Div for Metres {
    type Output = f64;
    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

impl Div<Seconds> for Metres {
    type Output = MetresPerSecond;
    fn div(self, rhs: Seconds) -> Self::Output {
        MetresPerSecond(self.0 / rhs.0)
    }
}

// ---- MetresPerSecond -----------------------------------------------------

impl MetresPerSecond {
    /// Speed in kilometres per hour.
    pub fn as_kmh(self) -> f64 {
        self.0 * 3.6
    }

    /// Construct from kilometres per hour.
    pub fn from_kmh(value: f64) -> Self {
        Self(value / 3.6)
    }
}

impl Mul<Seconds> for MetresPerSecond {
    type Output = Metres;
    fn mul(self, rhs: Seconds) -> Self::Output {
        Metres(self.0 * rhs.0)
    }
}

impl Neg for MetresPerSecond {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for MetresPerSecond {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for MetresPerSecond {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

// ---- Radians -------------------------------------------------------------

impl Radians {
    /// The same angle in degrees.
    pub fn as_degrees(self) -> f64 {
        self.0.to_degrees()
    }

    /// Wrap the angle into `[-π, π)`.
    pub fn wrap_pi(self) -> Self {
        let two_pi = core::f64::consts::TAU;
        let mut angle = self.0;
        // Use `rem_euclid` so negative values wrap the same way as positive ones.
        angle = (angle + core::f64::consts::PI).rem_euclid(two_pi) - core::f64::consts::PI;
        Self(angle)
    }
}

impl From<LatitudeDegrees> for Radians {
    fn from(value: LatitudeDegrees) -> Self {
        Self(value.0.to_radians())
    }
}

impl From<LongitudeDegrees> for Radians {
    fn from(value: LongitudeDegrees) -> Self {
        Self(value.0.to_radians())
    }
}

impl From<Radians> for LatitudeDegrees {
    fn from(value: Radians) -> Self {
        Self(value.0.to_degrees())
    }
}

impl From<Radians> for LongitudeDegrees {
    fn from(value: Radians) -> Self {
        Self(value.0.to_degrees())
    }
}

// ---- Display -------------------------------------------------------------

impl fmt::Display for Hertz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} s", self.0)
    }
}

impl fmt::Display for Metres {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} m", self.0)
    }
}

impl fmt::Display for MetresPerSecond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} m/s", self.0)
    }
}

impl fmt::Display for Radians {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} rad", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lat_lon_validation_accepts_the_bounds() {
        assert!(LatLon::new(90.0, 180.0).validate().is_ok());
        assert!(LatLon::new(-90.0, -180.0).validate().is_ok());
        assert!(LatLon::new(0.0, 0.0).validate().is_ok());
    }

    #[test]
    fn lat_lon_validation_rejects_out_of_range() {
        assert!(matches!(
            LatLon::new(90.1, 0.0).validate(),
            Err(CoreError::LatitudeOutOfRange(_))
        ));
        assert!(matches!(
            LatLon::new(0.0, 180.1).validate(),
            Err(CoreError::LongitudeOutOfRange(_))
        ));
    }

    #[test]
    fn lat_lon_validation_rejects_non_finite() {
        assert!(matches!(
            LatLon::new(f64::NAN, 0.0).validate(),
            Err(CoreError::NotFinite)
        ));
        assert!(matches!(
            LatLon::new(0.0, f64::INFINITY).validate(),
            Err(CoreError::NotFinite)
        ));
    }

    #[test]
    fn metres_per_second_times_seconds_is_metres() {
        let distance = MetresPerSecond(250.0) * Seconds(2.0);
        assert_eq!(distance, Metres(500.0));
    }

    #[test]
    fn metres_divided_by_seconds_is_speed() {
        let speed = Metres(500.0) / Seconds(2.0);
        assert_eq!(speed, MetresPerSecond(250.0));
    }

    #[test]
    fn radians_wrap_into_the_principal_interval() {
        let two_pi = core::f64::consts::TAU;
        for turns in -3..=3 {
            let angle = Radians(0.5 + f64::from(turns) * two_pi);
            let wrapped = angle.wrap_pi();
            assert!(
                (-core::f64::consts::PI..core::f64::consts::PI).contains(&wrapped.0),
                "{wrapped:?} not principal"
            );
            let delta = (wrapped.0 - 0.5).abs() % two_pi;
            assert!(
                delta < 1e-12 || (two_pi - delta) < 1e-12,
                "wrong representative"
            );
        }
    }

    #[test]
    fn seconds_round_trip_through_millis() {
        let value = Seconds(12.345);
        assert_eq!(value.as_millis_i64(), 12_345);
    }

    #[test]
    fn degree_radian_conversion_round_trips() {
        let original = 45.0;
        let rad = Radians::from(LatitudeDegrees(original));
        let back = LatitudeDegrees::from(rad).0;
        assert!((back - original).abs() < 1e-12);
    }
}
