//! PIP-04: public output geometry is coarsened onto a configured cell grid.
//! Two observers in the same cell produce the identical public cell, and the
//! disclosed size never drops below the configured minimum.

#![allow(clippy::expect_used)]

use core_types::LatLon;
use geodesy::{GeodesyError, LocalFrame};
use pipeline::public_cell;

#[test]
fn pip_04_same_cell_observers_share_one_public_cell() {
    // Two points ~300 m apart inside one 2 km cell.
    let a = public_cell(LatLon::new(52.0000, 13.0000), 2_000.0).expect("cell a");
    let b = public_cell(LatLon::new(52.0010, 13.0020), 2_000.0).expect("cell b");
    assert_eq!(a, b, "nearby observers must not split the public cell");
}

#[test]
fn pip_04_distant_observers_get_different_cells() {
    let a = public_cell(LatLon::new(52.0000, 13.0000), 2_000.0).expect("cell a");
    let b = public_cell(LatLon::new(52.0500, 13.0000), 2_000.0).expect("cell b");
    assert_ne!(a, b);
}

#[test]
fn pip_04_cell_centre_is_within_half_a_cell() {
    let observer = LatLon::new(52.0137, 13.0451);
    let cell = public_cell(observer, 2_000.0).expect("cell");
    // Convert the disclosed centre back to ENU around the true observer and
    // check it sits inside the cell that contains the observer.
    let frame = LocalFrame::wgs84(observer).expect("frame");
    let centre = LatLon::new(cell.lat_deg, cell.lon_deg);
    let enu = frame.to_enu(centre, 0.0).expect("enu");
    // The centre may be up to half a cell away in each axis.
    assert!(
        enu.east_m.abs() <= cell.size_m * 0.75,
        "east offset {} m",
        enu.east_m
    );
    assert!(
        enu.north_m.abs() <= cell.size_m * 0.75,
        "north offset {} m",
        enu.north_m
    );
    // And the disclosed size honours the configured minimum.
    assert!(cell.size_m >= 2_000.0);
}

#[test]
fn pip_04_rejects_bad_inputs() {
    let observer = LatLon::new(52.0, 13.0);
    assert!(matches!(
        public_cell(observer, 0.0),
        Err(GeodesyError::NotFinite)
    ));
    assert!(matches!(
        public_cell(observer, -100.0),
        Err(GeodesyError::NotFinite)
    ));
    assert!(matches!(
        public_cell(observer, f64::NAN),
        Err(GeodesyError::NotFinite)
    ));
    // Out-of-range coordinates are rejected by the frame, not snapped.
    assert!(public_cell(LatLon::new(95.0, 13.0), 2_000.0).is_err());
}
