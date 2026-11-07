//! Terrain elevation models and the observer-height datum (FR-DA-02).
//!
//! Public digital elevation models publish orthometric heights above the
//! geoid. SDD section 6.1 builds observer height from terrain plus a nominal
//! ear height, then adds the geoid undulation to reach the WGS84 ellipsoid.
//! [`DemGrid`] samples a regular grid of orthometric heights and
//! [`observer_height_m`] applies that datum exactly once.
//!
//! [`DemGrid`] reads the `doppler-dem` version 1 text format ([`DemGrid::parse`])
//! and SRTM `.hgt` bytes ([`DemGrid::from_srtm_hgt`]). A small attributed SRTM
//! crop ships under `crates/geodesy/tests/fixtures/` for the reference tests.

use core_types::{LatLon, Metres};
use serde::{Deserialize, Serialize};

use crate::error::GeodesyError;
use crate::geoid::GeoidModel;
use crate::grid::{self, GridGeometry};

/// A public digital elevation model.
///
/// Implementations wrap a local DEM cache. The trait is synchronous because
/// all library crates are synchronous (SDD section 3).
pub trait ElevationModel: std::fmt::Debug + Send + Sync {
    /// Return terrain elevation in metres above the geoid (orthometric
    /// height), which is the datum public DEMs publish.
    ///
    /// Ellipsoidal height is this value plus the [`crate::GeoidModel`]
    /// undulation at the same point; do not feed it to the ellipsoid as if it
    /// were already ellipsoidal, or the geoid correction is applied twice
    /// (SDD section 6.1).
    ///
    /// Returns [`GeodesyError::NoCoverage`] outside the model's bounds or
    /// where the model has a void.
    fn elevation_m(&self, position: LatLon) -> Result<f64, GeodesyError>;
}

/// Provenance and vertical accuracy of a terrain grid.
///
/// `product` and `version` identify the data; `source` and `licence` record
/// where it came from and under what terms (NFR-04, NFR-06).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemMetadata {
    /// Product name, for example `SRTM`.
    pub product: String,
    /// Product version, for example `SRTMGL1 v3 (1 arc-second)`.
    pub version: String,
    /// Where the grid came from, for example a tile and mirror.
    pub source: String,
    /// Licence or public-domain statement for the grid.
    pub licence: String,
    /// Published or measured vertical error in metres (SDD section 7).
    pub vertical_error_m: f64,
}

impl DemMetadata {
    /// Label for the manifest's model field, `"<product> <version>"`.
    ///
    /// Put this string in `core_types::ModelVersions::model` so a result can
    /// be tied to the exact terrain product that produced it (NFR-04).
    pub fn model_label(&self) -> String {
        format!("{} {}", self.product, self.version)
    }
}

impl std::fmt::Display for DemMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.model_label())
    }
}

/// A regular latitude/longitude grid of orthometric heights, in metres.
///
/// Row zero is the southern edge and column zero is the western edge; the
/// samples are row-major. `None` marks a void, which makes any cell that uses
/// it uncovered. The grid does not wrap across the antimeridian.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemGrid {
    metadata: DemMetadata,
    geometry: GridGeometry,
    elevations_m: Vec<Option<f64>>,
}

impl DemGrid {
    /// Build a grid from its provenance, geometry and row-major samples.
    ///
    /// Heights are metres above the geoid. `None` marks a void.
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedDem`] when a metadata field is empty,
    /// the vertical error is negative or not finite, the geometry is invalid,
    /// the sample count does not match `rows * cols`, or a sample is not
    /// finite.
    pub fn from_parts(
        metadata: DemMetadata,
        geometry: GridGeometry,
        elevations_m: Vec<Option<f64>>,
    ) -> Result<Self, GeodesyError> {
        validate_metadata(&metadata)?;
        geometry.validate().map_err(malformed)?;

        let expected = geometry
            .rows
            .checked_mul(geometry.cols)
            .ok_or_else(|| malformed("grid dimensions overflow"))?;
        if elevations_m.len() != expected {
            return Err(malformed("sample count does not match rows * cols"));
        }
        if elevations_m
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(malformed("elevations must all be finite or void"));
        }

        Ok(Self {
            metadata,
            geometry,
            elevations_m,
        })
    }

    /// Parse the `doppler-dem` version 1 text format.
    ///
    /// The file starts with `key=value` lines (blank lines and `#` comments
    /// are ignored), then a line containing only `elevation_m`, then
    /// `rows * cols` whitespace-separated heights in row-major order from south
    /// to north and west to east. A token of `void` marks a missing sample.
    /// Required keys are `product`, `version`, `source`, `licence`,
    /// `vertical_error_m`, `lat_min_deg`, `lon_min_deg`, `lat_step_deg`,
    /// `lon_step_deg`, `rows` and `cols`.
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedDem`] for an unknown or missing field,
    /// an unparsable number, a missing sample section, or any validation
    /// failure of [`DemGrid::from_parts`].
    pub fn parse(text: &str) -> Result<Self, GeodesyError> {
        const FIELDS: &[&str] = &[
            "product",
            "version",
            "source",
            "licence",
            "vertical_error_m",
            "lat_min_deg",
            "lon_min_deg",
            "lat_step_deg",
            "lon_step_deg",
            "rows",
            "cols",
        ];
        let (fields, samples) =
            grid::split_header(text, "elevation_m", FIELDS).map_err(malformed)?;
        let elevations_m = samples
            .into_iter()
            .map(|(token, line_number)| parse_sample(&token, line_number))
            .collect::<Result<Vec<_>, _>>()?;

        let metadata = DemMetadata {
            product: grid::field_text(&fields, "product").map_err(malformed)?,
            version: grid::field_text(&fields, "version").map_err(malformed)?,
            source: grid::field_text(&fields, "source").map_err(malformed)?,
            licence: grid::field_text(&fields, "licence").map_err(malformed)?,
            vertical_error_m: grid::field_number(&fields, "vertical_error_m").map_err(malformed)?,
        };
        Self::from_parts(
            metadata,
            GridGeometry {
                lat_min_deg: grid::field_number(&fields, "lat_min_deg").map_err(malformed)?,
                lon_min_deg: grid::field_number(&fields, "lon_min_deg").map_err(malformed)?,
                lat_step_deg: grid::field_number(&fields, "lat_step_deg").map_err(malformed)?,
                lon_step_deg: grid::field_number(&fields, "lon_step_deg").map_err(malformed)?,
                rows: grid::field_count(&fields, "rows").map_err(malformed)?,
                cols: grid::field_count(&fields, "cols").map_err(malformed)?,
            },
            elevations_m,
        )
    }

    /// Load one decompressed SRTM `.hgt` tile (FR-DA-02).
    ///
    /// SRTM stores signed 16-bit big-endian heights in metres above the EGM96
    /// geoid, one degree square, starting at the north-west corner. The tile
    /// name supplies the south-west corner; this function flips the rows into
    /// the south-to-north convention and maps the `-32768` void marker to
    /// `None`.
    ///
    /// `bytes` must be the raw `.hgt` payload, not a gzip stream. Callers that
    /// read `.hgt.gz` decompress first. Production SRTM tiles use 3600 samples
    /// per degree (3601 per side); the test suite uses small synthetic tiles.
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedDem`] when the byte length does not
    /// match `(samples_per_degree + 1)^2` 16-bit samples, the geometry is
    /// invalid, or the metadata fails validation.
    pub fn from_srtm_hgt(
        metadata: DemMetadata,
        tile: SrtmTile,
        bytes: &[u8],
    ) -> Result<Self, GeodesyError> {
        let side = tile
            .samples_per_degree
            .checked_add(1)
            .ok_or_else(|| malformed("SRTM sample count overflow"))?;
        let expected = side
            .checked_mul(side)
            .and_then(|samples| samples.checked_mul(2))
            .ok_or_else(|| malformed("SRTM sample count overflow"))?;
        if bytes.len() != expected {
            return Err(malformed(format!(
                "SRTM tile has {} bytes, expected {expected} for {} samples per degree",
                bytes.len(),
                tile.samples_per_degree
            )));
        }

        let sample_count = expected / 2;
        let mut elevations_m = vec![None; sample_count];
        for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            let raw = i16::from_be_bytes(*pair);
            let source_row = index / side;
            let col = index % side;
            let output_row = side - 1 - source_row;
            let output_index = output_row
                .checked_mul(side)
                .and_then(|offset| offset.checked_add(col))
                .ok_or_else(|| malformed("SRTM sample index overflow"))?;
            if let Some(slot) = elevations_m.get_mut(output_index) {
                *slot = if raw == i16::MIN {
                    None
                } else {
                    Some(f64::from(raw))
                };
            }
        }

        let step_deg = 1.0 / tile.samples_per_degree as f64;
        let geometry = GridGeometry {
            lat_min_deg: tile.lat_min_deg,
            lon_min_deg: tile.lon_min_deg,
            lat_step_deg: step_deg,
            lon_step_deg: step_deg,
            rows: side,
            cols: side,
        };
        Self::from_parts(metadata, geometry, elevations_m)
    }

    /// Provenance and vertical error recorded with this grid.
    pub fn metadata(&self) -> &DemMetadata {
        &self.metadata
    }

    /// Grid geometry: corner, spacing and dimensions.
    pub fn geometry(&self) -> GridGeometry {
        self.geometry
    }

    /// One sample in metres, if the indices are inside the grid and not void.
    fn sample_m(&self, row: usize, col: usize, position: LatLon) -> Result<f64, GeodesyError> {
        let index = self
            .geometry
            .sample_index(row, col)
            .ok_or_else(|| no_coverage(position))?;
        self.elevations_m
            .get(index)
            .copied()
            .flatten()
            .ok_or_else(|| no_coverage(position))
    }
}

impl ElevationModel for DemGrid {
    /// Bilinear terrain height at a coordinate, in metres above the geoid.
    ///
    /// A cell with any void corner returns [`GeodesyError::NoCoverage`].
    fn elevation_m(&self, position: LatLon) -> Result<f64, GeodesyError> {
        let (row, col) = self.geometry.grid_position(position)?;
        let row0 = self.geometry.lower_row(row);
        let col0 = self.geometry.lower_col(col);
        let row1 = row0 + 1;
        let col1 = col0 + 1;
        let row_fraction = row - row0 as f64;
        let col_fraction = col - col0 as f64;

        let south_west = self.sample_m(row0, col0, position)?;
        let north_west = self.sample_m(row1, col0, position)?;
        let south_east = self.sample_m(row0, col1, position)?;
        let north_east = self.sample_m(row1, col1, position)?;

        let south = south_west + col_fraction * (south_east - south_west);
        let north = north_west + col_fraction * (north_east - north_west);
        Ok(south + row_fraction * (north - south))
    }
}

/// One SRTM tile name and sampling density.
///
/// SRTM names give the south-west corner, for example `N27E086` covers
/// 27–28 N and 86–87 E. `samples_per_degree` is 3600 for the 1-arc-second
/// product and 1200 for the 3-arc-second product; a tile has one more sample
/// per side than this value because both edges are included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SrtmTile {
    lat_min_deg: f64,
    lon_min_deg: f64,
    samples_per_degree: usize,
}

impl SrtmTile {
    /// Parse an SRTM tile name such as `N27E086` or `S01W123`.
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedDem`] when the name is not
    /// `[NS]dd[EW]ddd`, a digit group is invalid, longitude 180 is marked east,
    /// latitude 90 is marked north, or `samples_per_degree` is zero.
    pub fn from_name(name: &str, samples_per_degree: usize) -> Result<Self, GeodesyError> {
        if samples_per_degree == 0 {
            return Err(malformed("SRTM samples per degree must be at least one"));
        }
        let mut chars = name.chars();
        let latitude_hemisphere = chars.next();
        let latitude_digits: String = chars.by_ref().take(2).collect();
        let longitude_hemisphere = chars.next();
        let longitude_digits: String = chars.collect();
        if latitude_digits.len() != 2 || longitude_digits.len() != 3 {
            return Err(malformed(format!(
                "SRTM tile name `{name}` is not [NS]dd[EW]ddd"
            )));
        }
        let latitude: u32 = latitude_digits
            .parse()
            .map_err(|_| malformed(format!("SRTM latitude in `{name}` is not a number")))?;
        let longitude: u32 = longitude_digits
            .parse()
            .map_err(|_| malformed(format!("SRTM longitude in `{name}` is not a number")))?;

        let lat_min_deg = match latitude_hemisphere {
            Some('N') if latitude <= 89 => f64::from(latitude),
            Some('S') if latitude <= 90 => -f64::from(latitude),
            _ => {
                return Err(malformed(format!(
                    "SRTM latitude in `{name}` is out of range"
                )));
            }
        };
        let lon_min_deg = match longitude_hemisphere {
            Some('E') if longitude <= 179 => f64::from(longitude),
            Some('W') if longitude <= 180 => -f64::from(longitude),
            _ => {
                return Err(malformed(format!(
                    "SRTM longitude in `{name}` is out of range"
                )));
            }
        };

        Ok(Self {
            lat_min_deg,
            lon_min_deg,
            samples_per_degree,
        })
    }

    /// Latitude of the south-west corner in degrees.
    pub fn lat_min_deg(&self) -> f64 {
        self.lat_min_deg
    }

    /// Longitude of the south-west corner in degrees.
    pub fn lon_min_deg(&self) -> f64 {
        self.lon_min_deg
    }

    /// Samples per degree of latitude and longitude.
    pub fn samples_per_degree(&self) -> usize {
        self.samples_per_degree
    }
}

/// Observer ellipsoidal height: terrain, ear height and geoid undulation.
///
/// Implements the observer-height rule of SDD section 6.1. `terrain` returns
/// orthometric height, `ear_height_m` is the nominal height of the ear above
/// the ground, and the geoid undulation converts the sum to the ellipsoid:
/// `h = H_terrain + H_ear + N`. All heights are metres.
///
/// # Errors
///
/// Returns [`GeodesyError::InvalidEarHeight`] when the ear height is negative
/// or not finite, and the sampling errors of `terrain` or `geoid`.
pub fn observer_height_m(
    terrain: &impl ElevationModel,
    geoid: &impl GeoidModel,
    position: LatLon,
    ear_height_m: Metres,
) -> Result<Metres, GeodesyError> {
    if !ear_height_m.0.is_finite() || ear_height_m.0 < 0.0 {
        return Err(GeodesyError::InvalidEarHeight(ear_height_m.0));
    }
    let terrain_m = terrain.elevation_m(position)?;
    let undulation_m = geoid.undulation_m(position)?;
    Ok(Metres(terrain_m + ear_height_m.0 + undulation_m))
}

/// Validate the provenance and vertical error of a terrain grid.
fn validate_metadata(metadata: &DemMetadata) -> Result<(), GeodesyError> {
    for (field, value) in [
        ("product", metadata.product.as_str()),
        ("version", metadata.version.as_str()),
        ("source", metadata.source.as_str()),
        ("licence", metadata.licence.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(malformed(format!("metadata {field} is empty")));
        }
    }
    if !metadata.vertical_error_m.is_finite() || metadata.vertical_error_m < 0.0 {
        return Err(malformed("vertical error must be finite and non-negative"));
    }
    Ok(())
}

/// Build the one DEM-format error.
fn malformed(message: impl Into<String>) -> GeodesyError {
    GeodesyError::MalformedDem(message.into())
}

/// Build the coverage error for a void or an out-of-grid sample.
fn no_coverage(position: LatLon) -> GeodesyError {
    GeodesyError::NoCoverage {
        lat_deg: position.lat_deg.0,
        lon_deg: position.lon_deg.0,
    }
}

/// Parse one sample, accepting `void` for a missing elevation.
fn parse_sample(raw: &str, line_number: usize) -> Result<Option<f64>, GeodesyError> {
    if raw.eq_ignore_ascii_case("void") {
        return Ok(None);
    }
    grid::parse_number(raw, "elevation_m", line_number)
        .map(Some)
        .map_err(malformed)
}
