//! Local East-North-Up frames (SDD section 6.1).

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use crate::ellipsoid::{Ecef, Ellipsoid, validate};
use crate::error::GeodesyError;

/// A local tangent frame anchored at one geodetic origin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalFrame {
    /// Geodetic origin of the frame.
    pub origin: LatLon,
    /// Reference ellipsoid.
    pub ellipsoid: Ellipsoid,
    /// Height of the origin above the ellipsoid in metres.
    pub origin_height_m: f64,
}

impl LocalFrame {
    /// Create a frame on WGS84 at an origin on the ellipsoid.
    pub fn wgs84(origin: LatLon) -> Result<Self, GeodesyError> {
        Self::wgs84_with_height(origin, 0.0)
    }

    /// Create a frame on WGS84 whose origin sits `height_m` above the
    /// ellipsoid.
    ///
    /// The observer's antenna usually sits on terrain, so its height matters
    /// for slant distances. Keeping it on the frame means every conversion is
    /// still a pure function of the frame's own fields.
    pub fn wgs84_with_height(origin: LatLon, height_m: f64) -> Result<Self, GeodesyError> {
        validate(origin)?;
        if !height_m.is_finite() {
            return Err(GeodesyError::NotFinite);
        }
        Ok(Self {
            origin,
            ellipsoid: Ellipsoid::WGS84,
            origin_height_m: height_m,
        })
    }

    /// Height of the frame origin above the ellipsoid in metres.
    pub fn origin_height_m(&self) -> f64 {
        self.origin_height_m
    }

    /// The frame origin in ECEF.
    pub fn origin_ecef(&self) -> Result<Ecef, GeodesyError> {
        self.ellipsoid.to_ecef(self.origin, self.origin_height_m)
    }

    /// Convert a geodetic position to local East-North-Up metres.
    pub fn to_enu(&self, position: LatLon, height_m: f64) -> Result<Enu, GeodesyError> {
        validate(position)?;
        let point_ecef = self.ellipsoid.to_ecef(position, height_m)?;
        self.ecef_to_enu(point_ecef)
    }

    /// Rotate an ECEF position into this frame's East-North-Up axes.
    pub fn ecef_to_enu(&self, position_ecef: Ecef) -> Result<Enu, GeodesyError> {
        if !position_ecef.is_finite() {
            return Err(GeodesyError::NotFinite);
        }
        let origin_ecef = self.origin_ecef()?;
        let d = position_ecef - origin_ecef;

        let lat = self.origin.lat_deg.0.to_radians();
        let lon = self.origin.lon_deg.0.to_radians();
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();

        Ok(Enu {
            east_m: -sin_lon * d.x_m + cos_lon * d.y_m,
            north_m: -sin_lat * cos_lon * d.x_m - sin_lat * sin_lon * d.y_m + cos_lat * d.z_m,
            up_m: cos_lat * cos_lon * d.x_m + cos_lat * sin_lon * d.y_m + sin_lat * d.z_m,
        })
    }

    /// Convert local East-North-Up metres back to an ECEF position.
    ///
    /// This is the transpose of the rotation used by [`Self::ecef_to_enu`], so
    /// the pair round-trips to floating-point precision.
    pub fn from_enu(&self, enu: Enu) -> Result<Ecef, GeodesyError> {
        if !enu.is_finite() {
            return Err(GeodesyError::NotFinite);
        }
        let origin_ecef = self.origin_ecef()?;

        let lat = self.origin.lat_deg.0.to_radians();
        let lon = self.origin.lon_deg.0.to_radians();
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();

        let e = enu.east_m;
        let n = enu.north_m;
        let u = enu.up_m;

        let dx = -sin_lon * e - sin_lat * cos_lon * n + cos_lat * cos_lon * u;
        let dy = cos_lon * e - sin_lat * sin_lon * n + cos_lat * sin_lon * u;
        let dz = cos_lat * n + sin_lat * u;

        Ok(origin_ecef + Ecef::new(dx, dy, dz))
    }

    /// Convert local East-North-Up metres back to a geodetic position and
    /// ellipsoidal height.
    pub fn enu_to_geodetic(&self, enu: Enu) -> Result<(LatLon, f64), GeodesyError> {
        let ecef = self.from_enu(enu)?;
        self.ellipsoid.from_ecef(ecef)
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

    /// Slant distance in metres.
    pub fn distance_m(&self) -> f64 {
        self.east_m.hypot(self.north_m).hypot(self.up_m)
    }

    /// Check that every component is finite.
    pub fn is_finite(&self) -> bool {
        self.east_m.is_finite() && self.north_m.is_finite() && self.up_m.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maximum permitted round-trip error in metres (sub-millimetre).
    const TOLERANCE_M: f64 = 1e-4;

    fn frame() -> LocalFrame {
        LocalFrame::wgs84_with_height(LatLon::new(51.477_9, -0.001_5), 46.0).expect("frame")
    }

    #[test]
    fn ecef_enu_round_trip_holds_for_local_offsets() {
        let frame = frame();
        let offsets = [
            Enu {
                east_m: 0.0,
                north_m: 0.0,
                up_m: 0.0,
            },
            Enu {
                east_m: 1_000.0,
                north_m: -2_500.0,
                up_m: 800.0,
            },
            Enu {
                east_m: -50_000.0,
                north_m: 75_000.0,
                up_m: 11_000.0,
            },
        ];
        for enu in offsets {
            let ecef = frame.from_enu(enu).expect("from_enu");
            let back = frame.ecef_to_enu(ecef).expect("ecef_to_enu");
            assert!(
                (back.east_m - enu.east_m).abs() < TOLERANCE_M,
                "{back:?} != {enu:?}",
            );
            assert!((back.north_m - enu.north_m).abs() < TOLERANCE_M);
            assert!((back.up_m - enu.up_m).abs() < TOLERANCE_M);
        }
    }

    #[test]
    fn geodetic_round_trip_holds_sub_millimetre() {
        let frame = frame();
        let target = LatLon::new(51.500_7, -0.124_6);
        let enu = frame.to_enu(target, 90.0).expect("to_enu");
        let (back, height) = frame.enu_to_geodetic(enu).expect("enu_to_geodetic");

        let expected = frame.ellipsoid.to_ecef(target, 90.0).expect("to_ecef");
        let actual = frame.ellipsoid.to_ecef(back, height).expect("to_ecef");
        assert!(actual.distance_m(expected) < TOLERANCE_M);
        assert!((height - 90.0).abs() < TOLERANCE_M);
    }

    #[test]
    fn frame_origin_maps_to_the_origin() {
        let frame = frame();
        let enu = frame
            .to_enu(frame.origin, frame.origin_height_m())
            .expect("to_enu");
        assert!(enu.distance_m() < TOLERANCE_M);
    }

    #[test]
    fn distance_helpers() {
        let enu = Enu {
            east_m: 3.0,
            north_m: 4.0,
            up_m: 12.0,
        };
        assert!((enu.horizontal_distance_m() - 5.0).abs() < 1e-12);
        assert!((enu.distance_m() - 13.0).abs() < 1e-12);
        let infinite = Enu {
            east_m: f64::INFINITY,
            ..Enu::default()
        };
        assert!(!infinite.is_finite());
    }

    #[test]
    fn rejects_non_finite_offsets() {
        let frame = frame();
        let bad = Enu {
            east_m: f64::NAN,
            ..Enu::default()
        };
        assert!(matches!(frame.from_enu(bad), Err(GeodesyError::NotFinite)));
    }
}
