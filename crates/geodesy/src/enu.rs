//! Local East-North-Up frames (SDD section 6.1).

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use crate::ellipsoid::{Ellipsoid, validate};
use crate::error::GeodesyError;

/// A local tangent frame anchored at one geodetic origin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalFrame {
    /// Geodetic origin of the frame.
    pub origin: LatLon,
    /// Reference ellipsoid.
    pub ellipsoid: Ellipsoid,
}

impl LocalFrame {
    /// Create a frame on WGS84.
    pub fn wgs84(origin: LatLon) -> Result<Self, GeodesyError> {
        validate(origin)?;
        Ok(Self {
            origin,
            ellipsoid: Ellipsoid::WGS84,
        })
    }

    /// Convert a geodetic position to local East-North-Up metres.
    pub fn to_enu(&self, position: LatLon, height_m: f64) -> Result<Enu, GeodesyError> {
        validate(position)?;
        let origin_ecef = self.ellipsoid.to_ecef(self.origin, 0.0)?;
        let point_ecef = self.ellipsoid.to_ecef(position, height_m)?;
        let dx = point_ecef.x_m - origin_ecef.x_m;
        let dy = point_ecef.y_m - origin_ecef.y_m;
        let dz = point_ecef.z_m - origin_ecef.z_m;

        let lat = self.origin.lat_deg.0.to_radians();
        let lon = self.origin.lon_deg.0.to_radians();
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();

        Ok(Enu {
            east_m: -sin_lon * dx + cos_lon * dy,
            north_m: -sin_lat * cos_lon * dx - sin_lat * sin_lon * dy + cos_lat * dz,
            up_m: cos_lat * cos_lon * dx + cos_lat * sin_lon * dy + sin_lat * dz,
        })
    }
}

/// A displacement in a local East-North-Up frame, in metres.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Enu {
    /// Eastward component in metres.
    pub east_m: f64,
    /// Northward component in metres.
    pub north_m: f64,
    /// Upward component in metres.
    pub up_m: f64,
}

impl Enu {
    /// Horizontal distance in metres.
    pub fn horizontal_distance_m(&self) -> f64 {
        self.east_m.hypot(self.north_m)
    }
}
