//! FR-DA-02: terrain sampling, SRTM loading and observer height.
//!
//! The fixture is a 33 by 33 one-arc-second crop of the SRTMGL1 tile
//! `N27E086` around Mount Everest, taken from the AWS Open Data terrain
//! tiles. SRTM is public domain; its provenance and source hash are in
//! `docs/third_party.md`.

#![allow(clippy::expect_used)]

use core_types::{LatLon, Metres};
use geodesy::{
    DemGrid, DemMetadata, ElevationModel, GeodesyError, GeoidGrid, GeoidMetadata, GeoidModel,
    GridGeometry, SrtmTile, observer_height_m,
};

const FIXTURE: &str = include_str!("fixtures/srtm_everest_33x33.txt");

fn fixture() -> DemGrid {
    DemGrid::parse(FIXTURE).expect("fixture parses")
}

fn metadata() -> DemMetadata {
    DemMetadata {
        product: "SRTM".to_owned(),
        version: "SRTMGL1 v3 (1 arc-second)".to_owned(),
        source: "AWS Open Data N27E086".to_owned(),
        licence: "Public domain (NASA/USGS)".to_owned(),
        vertical_error_m: 16.0,
    }
}

fn geometry(rows: usize, cols: usize) -> GridGeometry {
    GridGeometry {
        lat_min_deg: 0.0,
        lon_min_deg: 0.0,
        lat_step_deg: 1.0,
        lon_step_deg: 1.0,
        rows,
        cols,
    }
}

fn position_at(model: &DemGrid, row: f64, col: f64) -> LatLon {
    let geometry = model.geometry();
    LatLon::new(
        geometry.lat_min_deg + row * geometry.lat_step_deg,
        geometry.lon_min_deg + col * geometry.lon_step_deg,
    )
}

fn flat_grid(elevations_m: Vec<Option<f64>>) -> DemGrid {
    DemGrid::from_parts(metadata(), geometry(3, 3), elevations_m).expect("flat grid")
}

#[test]
fn fr_da_02_dem_bilinear_interpolation_matches_a_hand_computed_value() {
    let model = fixture();
    // South-west cell of the fixture. The four corners are metres above the
    // geoid, so the cell centre is their mean:
    // (7874 + 7888 + 7867 + 7897) / 4 = 7881.5.
    let expected = (7874.0 + 7888.0 + 7867.0 + 7897.0) / 4.0;
    let position = position_at(&model, 0.5, 0.5);

    let sampled = model.elevation_m(position).expect("covered cell centre");
    assert!(
        (sampled - expected).abs() < 1.0e-9,
        "bilinear sample {sampled} m vs hand value {expected} m"
    );
}

#[test]
fn fr_da_02_dem_reference_elevations_match_srtm() {
    let model = fixture();
    // SRTM node values from N27E086, metres above the EGM96 geoid.
    for (name, row, col, expected_m) in [
        ("south-west", 0.0, 0.0, 7874.0),
        ("north-west", 32.0, 0.0, 8174.0),
        ("south-east", 0.0, 32.0, 8346.0),
        ("north-east", 32.0, 32.0, 8553.0),
        ("centre", 16.0, 16.0, 8729.0),
    ] {
        let sampled = model
            .elevation_m(position_at(&model, row, col))
            .expect("covered node");
        assert!(
            (sampled - expected_m).abs() < 1.0e-9,
            "{name} node is {expected_m} m, sampled {sampled} m"
        );
    }
}

#[test]
fn fr_da_02_dem_grid_edges_are_inclusive() {
    let model = fixture();
    let geometry = model.geometry();
    let last_lat = geometry.lat_min_deg + (geometry.rows - 1) as f64 * geometry.lat_step_deg;
    let last_lon = geometry.lon_min_deg + (geometry.cols - 1) as f64 * geometry.lon_step_deg;
    assert!(model.elevation_m(LatLon::new(last_lat, last_lon)).is_ok());
}

#[test]
fn fr_da_02_dem_outside_coverage_and_voids_are_rejected() {
    let model = fixture();
    for position in [LatLon::new(27.0, 86.9), LatLon::new(28.1, 86.9)] {
        assert!(
            matches!(
                model.elevation_m(position),
                Err(GeodesyError::NoCoverage { .. })
            ),
            "{position:?} should be outside the fixture"
        );
    }

    let mut elevations = vec![Some(100.0); 9];
    if let Some(slot) = elevations.first_mut() {
        *slot = None;
    }
    let with_void = flat_grid(elevations);
    assert!(matches!(
        with_void.elevation_m(LatLon::new(0.0, 0.0)),
        Err(GeodesyError::NoCoverage { .. })
    ));
    assert!(with_void.elevation_m(LatLon::new(1.5, 1.5)).is_ok());
}

#[test]
fn fr_da_02_dem_metadata_records_product_version_and_vertical_error() {
    let model = fixture();
    let metadata = model.metadata();
    assert_eq!(metadata.product, "SRTM");
    assert!(metadata.version.contains("SRTMGL1"));
    assert!(metadata.source.contains("N27E086"));
    assert!(metadata.licence.contains("Public domain"));
    assert!((metadata.vertical_error_m - 16.0).abs() < 1.0e-12);
    assert_eq!(
        metadata.model_label(),
        "SRTM SRTMGL1 v3 (1 arc-second), 33x33 crop of N27E086"
    );
    assert_eq!(metadata.to_string(), metadata.model_label());
}

#[test]
fn fr_da_02_dem_srtm_hgt_loads_big_endian_and_flips_rows() {
    // A 5 by 5 synthetic tile at 4 samples per degree. SRTM stores rows from
    // north to south, so the first row is the north edge.
    let mut bytes = Vec::new();
    for value in 1_i16..=25 {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let tile = SrtmTile::from_name("N00E000", 4).expect("valid tile name");
    let model = DemGrid::from_srtm_hgt(metadata(), tile, &bytes).expect("tile loads");

    assert_eq!(model.geometry().rows, 5);
    assert_eq!(model.geometry().cols, 5);
    // The synthetic tile spans exactly one degree, 0 N to 1 N.
    assert!((model.elevation_m(LatLon::new(0.0, 0.0)).expect("south") - 21.0).abs() < 1.0e-12);
    assert!((model.elevation_m(LatLon::new(1.0, 0.0)).expect("north") - 1.0).abs() < 1.0e-12);
    assert!((model.elevation_m(LatLon::new(0.0, 1.0)).expect("east") - 25.0).abs() < 1.0e-12);

    let mut with_void = bytes.clone();
    if let Some(first) = with_void.first_mut() {
        *first = 0x80;
    }
    if let Some(second) = with_void.get_mut(1) {
        *second = 0x00;
    }
    let voided =
        DemGrid::from_srtm_hgt(metadata(), tile, &with_void).expect("tile with a void loads");
    assert!(matches!(
        voided.elevation_m(LatLon::new(1.0, 0.0)),
        Err(GeodesyError::NoCoverage { .. })
    ));
}

#[test]
fn fr_da_02_dem_srtm_hgt_rejects_bad_lengths() {
    let tile = SrtmTile::from_name("N27E086", 4).expect("valid tile name");
    for bytes in [&b""[..], &[0_u8; 49][..], &[0_u8; 51][..]] {
        assert!(
            matches!(
                DemGrid::from_srtm_hgt(metadata(), tile, bytes),
                Err(GeodesyError::MalformedDem(_))
            ),
            "{} bytes should be rejected",
            bytes.len()
        );
    }
}

#[test]
fn fr_da_02_dem_srtm_tile_names_parse_and_reject_bad_names() {
    let north_east = SrtmTile::from_name("N27E086", 3600).expect("valid name");
    assert!((north_east.lat_min_deg() - 27.0).abs() < 1.0e-12);
    assert!((north_east.lon_min_deg() - 86.0).abs() < 1.0e-12);
    assert_eq!(north_east.samples_per_degree(), 3600);

    let south_west = SrtmTile::from_name("S01W123", 1200).expect("valid name");
    assert!((south_west.lat_min_deg() + 1.0).abs() < 1.0e-12);
    assert!((south_west.lon_min_deg() + 123.0).abs() < 1.0e-12);

    for name in [
        "", "N27E08", "N27E0861", "X27E086", "N27X086", "N90E086", "N27E180",
    ] {
        assert!(
            matches!(
                SrtmTile::from_name(name, 3600),
                Err(GeodesyError::MalformedDem(_))
            ),
            "`{name}` should be rejected"
        );
    }
    assert!(matches!(
        SrtmTile::from_name("N27E086", 0),
        Err(GeodesyError::MalformedDem(_))
    ));
}

#[test]
fn fr_da_02_observer_height_combines_terrain_ear_and_geoid() {
    let terrain = fixture();
    let geoid = GeoidGrid::from_parts(
        GeoidMetadata {
            name: "constant".to_owned(),
            version: "test".to_owned(),
            source: "test".to_owned(),
            licence: "test".to_owned(),
        },
        GridGeometry {
            lat_min_deg: 27.0,
            lon_min_deg: 86.0,
            lat_step_deg: 1.0,
            lon_step_deg: 1.0,
            rows: 2,
            cols: 2,
        },
        vec![2.0; 4],
    )
    .expect("constant geoid");
    let position = position_at(&terrain, 0.0, 0.0);

    let height = observer_height_m(&terrain, &geoid, position, Metres(1.5)).expect("height");
    // SDD 6.1: h = H_terrain + H_ear + N.
    assert!(
        (height.0 - (7874.0 + 1.5 + 2.0)).abs() < 1.0e-9,
        "{height:?}"
    );
    assert!(!geoid.undulation_m(position).expect("undulation").is_nan());

    for bad in [-0.5, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            observer_height_m(&terrain, &geoid, position, Metres(bad)),
            Err(GeodesyError::InvalidEarHeight(_))
        ));
    }
}

#[test]
fn fr_da_02_dem_malformed_grids_are_rejected() {
    let metadata = metadata();
    let values = vec![Some(0.0); 9];

    assert!(matches!(
        DemGrid::from_parts(
            DemMetadata {
                product: String::new(),
                ..metadata.clone()
            },
            geometry(3, 3),
            values.clone()
        ),
        Err(GeodesyError::MalformedDem(_))
    ));
    assert!(matches!(
        DemGrid::from_parts(
            DemMetadata {
                vertical_error_m: -1.0,
                ..metadata.clone()
            },
            geometry(3, 3),
            values.clone()
        ),
        Err(GeodesyError::MalformedDem(_))
    ));
    assert!(matches!(
        DemGrid::from_parts(
            metadata.clone(),
            GridGeometry {
                lat_step_deg: 0.0,
                ..geometry(3, 3)
            },
            values.clone()
        ),
        Err(GeodesyError::MalformedDem(_))
    ));
    assert!(matches!(
        DemGrid::from_parts(metadata.clone(), geometry(3, 3), vec![Some(0.0); 8]),
        Err(GeodesyError::MalformedDem(_))
    ));
    let mut not_finite = values;
    if let Some(slot) = not_finite.first_mut() {
        *slot = Some(f64::INFINITY);
    }
    assert!(matches!(
        DemGrid::from_parts(metadata, geometry(3, 3), not_finite),
        Err(GeodesyError::MalformedDem(_))
    ));
}

#[test]
fn fr_da_02_dem_malformed_text_is_rejected_with_a_reason() {
    let missing_section = FIXTURE.replace("elevation_m", "samples");
    assert!(matches!(
        DemGrid::parse(&missing_section),
        Err(GeodesyError::MalformedDem(_))
    ));

    let missing_product = FIXTURE.replace("product=SRTM\n", "");
    assert!(matches!(
        DemGrid::parse(&missing_product),
        Err(GeodesyError::MalformedDem(_))
    ));

    let unknown_field = FIXTURE.replace("product=SRTM", "product=SRTM\ncolour=blue");
    assert!(matches!(
        DemGrid::parse(&unknown_field),
        Err(GeodesyError::MalformedDem(_))
    ));

    let bad_number = FIXTURE.replace("vertical_error_m=16.0", "vertical_error_m=sixteen");
    assert!(matches!(
        DemGrid::parse(&bad_number),
        Err(GeodesyError::MalformedDem(_))
    ));

    let void_text = FIXTURE.replacen("7874", "void", 1);
    let with_void = DemGrid::parse(&void_text).expect("void token is accepted");
    assert!(matches!(
        with_void.elevation_m(position_at(&with_void, 0.0, 0.0)),
        Err(GeodesyError::NoCoverage { .. })
    ));
}
