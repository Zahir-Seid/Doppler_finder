//! FR-DA-02: geoid sampling and height conversion.
//!
//! The fixture is a 13 by 13 subset of the NGA EGM96 15-minute geoid over
//! 50–53 N, 2 W–1 E, extracted from
//! `https://download.osgeo.org/proj/vdatum/egm96_15/egm96_15.gtx`. The grid is
//! public domain; its provenance is in `docs/third_party.md`.

#![allow(clippy::expect_used)]

use core_types::{LatLon, Metres};
use geodesy::{GeodesyError, GeoidGrid, GeoidMetadata, GeoidModel, GridGeometry};

const FIXTURE: &str = include_str!("fixtures/egm96_uk_3deg.txt");

fn fixture() -> GeoidGrid {
    GeoidGrid::parse(FIXTURE).expect("fixture parses")
}

#[test]
fn fr_da_02_bilinear_interpolation_matches_a_hand_computed_value() {
    let model = fixture();
    // Cell corners at 50.0 N / 50.25 N and 2.0 W / 1.75 W, from the fixture.
    let south_west = 48.049;
    let north_west = 47.878;
    let south_east = 47.475;
    let north_east = 47.269;
    let expected = (south_west + north_west + south_east + north_east) / 4.0;

    let sampled = model
        .undulation_m(LatLon::new(50.125, -1.875))
        .expect("covered point");
    assert!(
        (sampled - expected).abs() < 1.0e-9,
        "bilinear sample {sampled} vs hand value {expected}"
    );
}

#[test]
fn fr_da_02_reference_undulations_match_egm96() {
    let model = fixture();
    // Node values from the EGM96 15-minute grid. The fixture stores three
    // decimals, hence the millimetre tolerance.
    for (lat_deg, lon_deg, expected_m) in [
        (50.0, -2.0, 48.048_809),
        (51.5, 0.0, 45.817_898),
        (52.25, -1.25, 48.465_454),
        (53.0, 1.0, 45.012_741),
    ] {
        let sampled = model
            .undulation_m(LatLon::new(lat_deg, lon_deg))
            .expect("covered node");
        assert!(
            (sampled - expected_m).abs() < 1.0e-3,
            "EGM96 at ({lat_deg}, {lon_deg}) is {expected_m} m, sampled {sampled} m"
        );
    }
}

#[test]
fn fr_da_02_orthometric_and_ellipsoidal_heights_round_trip() {
    let model = fixture();
    for (lat_deg, lon_deg) in [(50.0, -2.0), (51.5, 0.0), (53.0, 1.0)] {
        let position = LatLon::new(lat_deg, lon_deg);
        let undulation_m = model.undulation_m(position).expect("covered point");
        let orthometric_m = Metres(120.0);

        let ellipsoidal_m = model
            .orthometric_to_ellipsoidal(position, orthometric_m)
            .expect("conversion");
        assert!(
            (ellipsoidal_m.0 - (orthometric_m.0 + undulation_m)).abs() < 1.0e-12,
            "orthometric {orthometric_m:?} plus {undulation_m} gave {ellipsoidal_m:?}"
        );

        let back = model
            .ellipsoidal_to_orthometric(position, ellipsoidal_m)
            .expect("inverse conversion");
        assert!(
            (back.0 - orthometric_m.0).abs() < 1.0e-12,
            "round trip moved {orthometric_m:?} to {back:?}"
        );
    }
}

#[test]
fn fr_da_02_outside_coverage_and_bad_coordinates_are_rejected() {
    let model = fixture();
    for position in [
        LatLon::new(49.9, -1.0),
        LatLon::new(53.1, -1.0),
        LatLon::new(51.0, -2.1),
        LatLon::new(51.0, 1.1),
    ] {
        assert!(
            matches!(
                model.undulation_m(position),
                Err(GeodesyError::NoCoverage { .. })
            ),
            "{position:?} should be outside the grid"
        );
    }

    assert!(matches!(
        model.undulation_m(LatLon::new(91.0, 0.0)),
        Err(GeodesyError::LatitudeOutOfRange(_))
    ));
    assert!(matches!(
        model.ellipsoidal_to_orthometric(LatLon::new(51.0, 0.0), Metres(f64::NAN)),
        Err(GeodesyError::NotFinite)
    ));
}

#[test]
fn fr_da_02_metadata_records_the_model_version_and_source() {
    let model = fixture();
    let metadata = model.metadata();
    assert_eq!(metadata.name, "EGM96");
    assert_eq!(metadata.version, "15-minute grid (1996)");
    assert!(metadata.source.contains("egm96_15.gtx"));
    assert!(metadata.licence.contains("Public domain"));
    assert_eq!(metadata.model_label(), "EGM96 15-minute grid (1996)");
    assert_eq!(metadata.to_string(), metadata.model_label());
    assert_eq!(model.rows(), 13);
    assert_eq!(model.cols(), 13);
}

#[test]
fn fr_da_02_malformed_grids_are_rejected() {
    let metadata = fixture().metadata().clone();
    let values = vec![0.0; 9];
    let geometry = GridGeometry {
        lat_min_deg: 0.0,
        lon_min_deg: 0.0,
        lat_step_deg: 1.0,
        lon_step_deg: 1.0,
        rows: 3,
        cols: 3,
    };

    assert!(matches!(
        GeoidGrid::from_parts(
            GeoidMetadata {
                name: String::new(),
                ..metadata.clone()
            },
            geometry,
            values.clone()
        ),
        Err(GeodesyError::MalformedGeoidGrid(_))
    ));
    assert!(matches!(
        GeoidGrid::from_parts(
            metadata.clone(),
            GridGeometry {
                lat_step_deg: 0.0,
                ..geometry
            },
            values.clone()
        ),
        Err(GeodesyError::MalformedGeoidGrid(_))
    ));
    assert!(matches!(
        GeoidGrid::from_parts(metadata.clone(), geometry, vec![0.0; 8]),
        Err(GeodesyError::MalformedGeoidGrid(_))
    ));
    let mut not_finite = values;
    if let Some(first) = not_finite.first_mut() {
        *first = f64::INFINITY;
    }
    assert!(matches!(
        GeoidGrid::from_parts(metadata, geometry, not_finite),
        Err(GeodesyError::MalformedGeoidGrid(_))
    ));
}

#[test]
fn fr_da_02_malformed_text_is_rejected_with_a_reason() {
    let missing_section = FIXTURE.replace("undulation_m", "samples");
    assert!(matches!(
        GeoidGrid::parse(&missing_section),
        Err(GeodesyError::MalformedGeoidGrid(message)) if message.contains("undulation_m")
    ));

    let unknown_field = FIXTURE.replace("name=EGM96", "product=EGM96");
    assert!(matches!(
        GeoidGrid::parse(&unknown_field),
        Err(GeodesyError::MalformedGeoidGrid(message)) if message.contains("product")
    ));

    let bad_number = FIXTURE.replace("lat_step_deg=0.25", "lat_step_deg=north");
    assert!(matches!(
        GeoidGrid::parse(&bad_number),
        Err(GeodesyError::MalformedGeoidGrid(message)) if message.contains("lat_step_deg")
    ));
}

#[test]
fn fr_da_02_grid_edges_are_inclusive() {
    let model = fixture();
    for position in [
        LatLon::new(50.0, -2.0),
        LatLon::new(53.0, 1.0),
        LatLon::new(50.0, 1.0),
        LatLon::new(53.0, -2.0),
    ] {
        assert!(
            model.undulation_m(position).is_ok(),
            "corner {position:?} should be covered"
        );
    }
}
