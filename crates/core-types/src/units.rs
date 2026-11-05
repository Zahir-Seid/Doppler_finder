//! Physical quantity newtypes.
//!
//! Units are part of the type, never a comment or a bare `f64` (Norms section
//! 5). Degrees are converted to radians at the geodesy boundary; radians are
//! used for every internal angle.

use serde::{Deserialize, Serialize};

/// A frequency in hertz.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hertz(pub f64);

/// A time interval or duration in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seconds(pub f64);

/// A length in metres.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Metres(pub f64);

/// A speed in metres per second.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MetresPerSecond(pub f64);

/// An angle in radians.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Radians(pub f64);

/// A latitude in degrees, positive north.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LatitudeDegrees(pub f64);

/// A longitude in degrees, positive east.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
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
