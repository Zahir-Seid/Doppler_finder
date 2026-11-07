//! WGS84 coordinates, ECEF and local ENU frames, and terrain/geoid traits.
//!
//! The implementation follows `docs/SDD.md` sections 4 and 6.1. Geodetic
//! latitude and longitude enter as degrees only at this boundary.

pub mod dem;
pub mod ellipsoid;
pub mod enu;
pub mod error;
pub mod geoid;
pub mod grid;

pub use dem::{DemGrid, DemMetadata, ElevationModel, SrtmTile, observer_height_m};
pub use ellipsoid::{Ecef, Ellipsoid};
pub use enu::{Enu, LocalFrame};
pub use error::GeodesyError;
pub use geoid::{GeoidGrid, GeoidMetadata, GeoidModel};
pub use grid::GridGeometry;
