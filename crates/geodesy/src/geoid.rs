//! Geoid undulation models (FR-DA-02).
//!
//! `docs/SDD.md` section 6.1 turns a terrain height into an ellipsoidal height
//! by adding the geoid undulation, because public digital elevation models
//! publish orthometric heights above the geoid while the propagation maths
//! needs heights above the WGS84 ellipsoid. Section 9.4 also requires the
//! model name and version in the run manifest (NFR-04), so every grid carries
//! its provenance next to its samples.
//!
//! [`GeoidGrid`] is a regular latitude/longitude grid with bilinear sampling
//! and no external dependency. Its text format is documented on
//! [`GeoidGrid::parse`]; a small attributed EGM96 subset ships under
//! `crates/geodesy/tests/fixtures/` for the reference tests.

use core_types::{LatLon, Metres};
use serde::{Deserialize, Serialize};

use crate::error::GeodesyError;
use crate::grid::{self, GridGeometry};

/// A geoid model converting ellipsoidal height to orthometric height.
pub trait GeoidModel: std::fmt::Debug + Send + Sync {
    /// Return the geoid undulation in metres at a geodetic position.
    ///
    /// The geometric altitude used by the estimator is the orthometric
    /// altitude plus this undulation (SDD section 6.1).
    ///
    /// Returns [`GeodesyError::NoCoverage`] outside the model's bounds and
    /// [`GeodesyError::NotFinite`] or an out-of-range error for an invalid
    /// coordinate.
    fn undulation_m(&self, position: LatLon) -> Result<f64, GeodesyError>;

    /// Convert an orthometric height to an ellipsoidal height, in metres.
    ///
    /// This is `H + N` from SDD section 6.1, where `H` is the height above
    /// the geoid and `N` is [`GeoidModel::undulation_m`]. Both inputs and the
    /// result are metres.
    fn orthometric_to_ellipsoidal(
        &self,
        position: LatLon,
        orthometric_m: Metres,
    ) -> Result<Metres, GeodesyError> {
        if !orthometric_m.0.is_finite() {
            return Err(GeodesyError::NotFinite);
        }
        Ok(Metres(orthometric_m.0 + self.undulation_m(position)?))
    }

    /// Convert an ellipsoidal height to an orthometric height, in metres.
    ///
    /// This is `H = h - N` from SDD section 6.1, the inverse of
    /// [`GeoidModel::orthometric_to_ellipsoidal`].
    fn ellipsoidal_to_orthometric(
        &self,
        position: LatLon,
        ellipsoidal_m: Metres,
    ) -> Result<Metres, GeodesyError> {
        if !ellipsoidal_m.0.is_finite() {
            return Err(GeodesyError::NotFinite);
        }
        Ok(Metres(ellipsoidal_m.0 - self.undulation_m(position)?))
    }
}

/// Provenance of a geoid grid, copied into the run manifest (NFR-04).
///
/// `name` and `version` identify the model; `source` and `licence` record
/// where the samples came from and under what terms (NFR-06).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoidMetadata {
    /// Model name, for example `EGM96`.
    pub name: String,
    /// Model version, for example `15-minute grid (1996)`.
    pub version: String,
    /// Where the grid came from, for example a product and file name.
    pub source: String,
    /// Licence or public-domain statement for the grid.
    pub licence: String,
}

impl GeoidMetadata {
    /// Label for the manifest's model field, `"<name> <version>"`.
    ///
    /// Put this string in `core_types::ModelVersions::model` so a result can
    /// be tied to the exact geoid model that produced it (NFR-04).
    pub fn model_label(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

impl std::fmt::Display for GeoidMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.model_label())
    }
}

/// A regular latitude/longitude grid of geoid undulations, in metres.
///
/// Row zero is the southern edge and column zero is the western edge; the
/// samples are row-major. Latitude and longitude steps are positive and the
/// grid does not wrap across the antimeridian: a model that crosses it must be
/// supplied as one grid whose longitudes stay within `[-180, 180]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoidGrid {
    metadata: GeoidMetadata,
    geometry: GridGeometry,
    undulations_m: Vec<f64>,
}

impl GeoidGrid {
    /// Build a grid from its provenance, geometry and row-major samples.
    ///
    /// Undulations are metres above the WGS84 ellipsoid, positive where the
    /// geoid lies above it (SDD section 6.1).
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedGeoidGrid`] when a metadata field is
    /// empty, a dimension is below two, a step is not positive, an edge lies
    /// outside `[-90, 90]` or `[-180, 180]` degrees, the sample count does not
    /// match `rows * cols`, or a sample is not finite.
    pub fn from_parts(
        metadata: GeoidMetadata,
        geometry: GridGeometry,
        undulations_m: Vec<f64>,
    ) -> Result<Self, GeodesyError> {
        validate_metadata(&metadata)?;
        geometry.validate().map_err(malformed)?;

        let expected = geometry
            .rows
            .checked_mul(geometry.cols)
            .ok_or_else(|| malformed("grid dimensions overflow"))?;
        if undulations_m.len() != expected {
            return Err(malformed("sample count does not match rows * cols"));
        }
        if undulations_m.iter().any(|value| !value.is_finite()) {
            return Err(malformed("undulations must all be finite"));
        }

        Ok(Self {
            metadata,
            geometry,
            undulations_m,
        })
    }

    /// Parse the `doppler-geoid` version 1 text format.
    ///
    /// The file starts with `key=value` lines (blank lines and `#` comments
    /// are ignored), then a line containing only `undulation_m`, then
    /// `rows * cols` whitespace-separated metre values in row-major order from
    /// south to north and west to east. Required keys are `name`, `version`,
    /// `source`, `licence`, `lat_min_deg`, `lon_min_deg`, `lat_step_deg`,
    /// `lon_step_deg`, `rows` and `cols`.
    ///
    /// # Errors
    ///
    /// Returns [`GeodesyError::MalformedGeoidGrid`] for an unknown or missing
    /// field, an unparsable number, a missing sample section, or any
    /// validation failure of [`GeoidGrid::from_parts`].
    pub fn parse(text: &str) -> Result<Self, GeodesyError> {
        const FIELDS: &[&str] = &[
            "name",
            "version",
            "source",
            "licence",
            "lat_min_deg",
            "lon_min_deg",
            "lat_step_deg",
            "lon_step_deg",
            "rows",
            "cols",
        ];
        let (fields, samples) =
            grid::split_header(text, "undulation_m", FIELDS).map_err(malformed)?;
        let undulations_m = samples
            .into_iter()
            .map(|(token, line_number)| {
                grid::parse_number(&token, "undulation_m", line_number).map_err(malformed)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let metadata = GeoidMetadata {
            name: grid::field_text(&fields, "name").map_err(malformed)?,
            version: grid::field_text(&fields, "version").map_err(malformed)?,
            source: grid::field_text(&fields, "source").map_err(malformed)?,
            licence: grid::field_text(&fields, "licence").map_err(malformed)?,
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
            undulations_m,
        )
    }

    /// Provenance recorded with this grid (NFR-04, NFR-06).
    pub fn metadata(&self) -> &GeoidMetadata {
        &self.metadata
    }

    /// Grid geometry: corner, spacing and dimensions.
    pub fn geometry(&self) -> GridGeometry {
        self.geometry
    }

    /// Number of latitude samples.
    pub fn rows(&self) -> usize {
        self.geometry.rows
    }

    /// Number of longitude samples.
    pub fn cols(&self) -> usize {
        self.geometry.cols
    }

    /// South-west corner latitude in degrees.
    pub fn lat_min_deg(&self) -> f64 {
        self.geometry.lat_min_deg
    }

    /// South-west corner longitude in degrees.
    pub fn lon_min_deg(&self) -> f64 {
        self.geometry.lon_min_deg
    }

    /// Latitude spacing in degrees.
    pub fn lat_step_deg(&self) -> f64 {
        self.geometry.lat_step_deg
    }

    /// Longitude spacing in degrees.
    pub fn lon_step_deg(&self) -> f64 {
        self.geometry.lon_step_deg
    }

    /// One sample in metres, if the indices are inside the grid.
    fn sample_m(&self, row: usize, col: usize) -> Result<f64, GeodesyError> {
        let index = self
            .geometry
            .sample_index(row, col)
            .ok_or_else(|| malformed("sample index outside the grid"))?;
        self.undulations_m
            .get(index)
            .copied()
            .ok_or_else(|| malformed("sample index outside the grid"))
    }
}

impl GeoidModel for GeoidGrid {
    /// Bilinear undulation at a coordinate, in metres (SDD section 6.1).
    ///
    /// Cells are interpolated linearly in latitude and longitude. The grid
    /// edge is inclusive within floating-point noise; anything beyond it
    /// returns [`GeodesyError::NoCoverage`].
    fn undulation_m(&self, position: LatLon) -> Result<f64, GeodesyError> {
        let (row, col) = self.geometry.grid_position(position)?;
        let row0 = self.geometry.lower_row(row);
        let col0 = self.geometry.lower_col(col);
        let row1 = row0 + 1;
        let col1 = col0 + 1;
        let row_fraction = row - row0 as f64;
        let col_fraction = col - col0 as f64;

        let south_west = self.sample_m(row0, col0)?;
        let north_west = self.sample_m(row1, col0)?;
        let south_east = self.sample_m(row0, col1)?;
        let north_east = self.sample_m(row1, col1)?;

        let south = south_west + col_fraction * (south_east - south_west);
        let north = north_west + col_fraction * (north_east - north_west);
        Ok(south + row_fraction * (north - south))
    }
}

/// Validate the provenance every grid must carry.
fn validate_metadata(metadata: &GeoidMetadata) -> Result<(), GeodesyError> {
    if metadata.name.trim().is_empty() {
        return Err(malformed("metadata name is empty"));
    }
    if metadata.version.trim().is_empty() {
        return Err(malformed("metadata version is empty"));
    }
    if metadata.source.trim().is_empty() {
        return Err(malformed("metadata source is empty"));
    }
    if metadata.licence.trim().is_empty() {
        return Err(malformed("metadata licence is empty"));
    }
    Ok(())
}

/// Build the one grid-format error.
fn malformed(message: impl Into<String>) -> GeodesyError {
    GeodesyError::MalformedGeoidGrid(message.into())
}
