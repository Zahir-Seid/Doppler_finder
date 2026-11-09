//! Public-geometry coarsening (PIP-04, FR-OUT-03, D-1).
//!
//! This module is the *only* place public output geometry is constructed.
//! [`public_cell`] snaps a refined observer position onto a local
//! East-North-Up grid whose cells are squares of at least the configured
//! side length, so two nearby estimates cannot disclose sub-cell precision.
//! The result is a [`PublicCell`] — a centre and a size — never a bare
//! coordinate pair.

use core_types::LatLon;
use geodesy::{Enu, GeodesyError, LocalFrame};

use crate::result::PublicCell;

/// Snap `observer` to the centre of its public output cell.
///
/// The cell grid is anchored at the observer's integer-degree position, not
/// at the observer itself — anchoring at the observer would make every point
/// its own cell centre and leak full precision. Within one degree the grid
/// is fixed, so two observers in the same cell produce the identical
/// [`PublicCell`]; across a degree boundary the grids realign, which is
/// acceptable because each side still discloses only its own cell.
///
/// # Errors
///
/// Returns [`GeodesyError`] for an invalid coordinate or a non-positive or
/// non-finite cell size.
pub fn public_cell(observer: LatLon, cell_size_m: f64) -> Result<PublicCell, GeodesyError> {
    if !cell_size_m.is_finite() || cell_size_m <= 0.0 {
        return Err(GeodesyError::NotFinite);
    }
    let anchor = LatLon::new(observer.lat_deg.0.floor(), observer.lon_deg.0.floor());
    let frame = LocalFrame::wgs84(anchor)?;
    let enu = frame.to_enu(observer, 0.0)?;
    // Snap to the nearest cell centre in each axis.
    let centre_east = (enu.east_m / cell_size_m).round() * cell_size_m;
    let centre_north = (enu.north_m / cell_size_m).round() * cell_size_m;
    let (centre, _height) = frame.enu_to_geodetic(Enu {
        east_m: centre_east,
        north_m: centre_north,
        up_m: 0.0,
    })?;
    Ok(PublicCell {
        lat_deg: centre.lat_deg.0,
        lon_deg: centre.lon_deg.0,
        size_m: cell_size_m,
    })
}
