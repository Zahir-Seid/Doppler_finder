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

/// Fraction of a grid cell a coordinate may fall outside the last sample and
/// still count as covered. It absorbs floating-point noise at a grid edge.
const EDGE_TOLERANCE_CELLS: f64 = 1.0e-9;

/// Angular slack when validating a grid's north or east edge, in degrees.
const EDGE_TOLERANCE_DEG: f64 = 1.0e-9;

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

/// Geometry of a regular latitude/longitude grid.
///
/// `lat_min_deg` and `lon_min_deg` are the southern and western edges; the
/// steps are positive degrees between neighbouring samples. Row zero is the
/// southern edge and column zero is the western edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoidGeometry {
    /// Latitude of the southern edge in degrees.
    pub lat_min_deg: f64,
    /// Longitude of the western edge in degrees.
    pub lon_min_deg: f64,
    /// Positive latitude spacing in degrees.
    pub lat_step_deg: f64,
    /// Positive longitude spacing in degrees.
    pub lon_step_deg: f64,
    /// Number of latitude samples, at least two.
    pub rows: usize,
    /// Number of longitude samples, at least two.
    pub cols: usize,
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
    geometry: GeoidGeometry,
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
        geometry: GeoidGeometry,
        undulations_m: Vec<f64>,
    ) -> Result<Self, GeodesyError> {
        validate_metadata(&metadata)?;
        validate_geometry(geometry)?;

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
        let mut name = None;
        let mut version = None;
        let mut source = None;
        let mut licence = None;
        let mut lat_min_deg = None;
        let mut lon_min_deg = None;
        let mut lat_step_deg = None;
        let mut lon_step_deg = None;
        let mut rows = None;
        let mut cols = None;
        let mut undulations_m = Vec::new();
        let mut in_samples = false;

        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if in_samples {
                for token in line.split_whitespace() {
                    undulations_m.push(parse_number(token, "undulation_m", line_number)?);
                }
                continue;
            }
            if line == "undulation_m" {
                in_samples = true;
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(malformed(format!(
                    "line {line_number}: expected `key=value` or `undulation_m`"
                )));
            };
            let key = raw_key.trim();
            let value = raw_value.trim();
            match key {
                "name" => name = Some(require_non_empty(value, key, line_number)?),
                "version" => version = Some(require_non_empty(value, key, line_number)?),
                "source" => source = Some(require_non_empty(value, key, line_number)?),
                "licence" => licence = Some(require_non_empty(value, key, line_number)?),
                "lat_min_deg" => {
                    lat_min_deg = Some(parse_number(value, key, line_number)?);
                }
                "lon_min_deg" => {
                    lon_min_deg = Some(parse_number(value, key, line_number)?);
                }
                "lat_step_deg" => {
                    lat_step_deg = Some(parse_number(value, key, line_number)?);
                }
                "lon_step_deg" => {
                    lon_step_deg = Some(parse_number(value, key, line_number)?);
                }
                "rows" => rows = Some(parse_count(value, key, line_number)?),
                "cols" => cols = Some(parse_count(value, key, line_number)?),
                other => {
                    return Err(malformed(format!(
                        "line {line_number}: unknown field `{other}`"
                    )));
                }
            }
        }

        if !in_samples {
            return Err(malformed("missing `undulation_m` sample section"));
        }

        let metadata = GeoidMetadata {
            name: require(name, "name")?,
            version: require(version, "version")?,
            source: require(source, "source")?,
            licence: require(licence, "licence")?,
        };
        Self::from_parts(
            metadata,
            GeoidGeometry {
                lat_min_deg: require(lat_min_deg, "lat_min_deg")?,
                lon_min_deg: require(lon_min_deg, "lon_min_deg")?,
                lat_step_deg: require(lat_step_deg, "lat_step_deg")?,
                lon_step_deg: require(lon_step_deg, "lon_step_deg")?,
                rows: require(rows, "rows")?,
                cols: require(cols, "cols")?,
            },
            undulations_m,
        )
    }

    /// Provenance recorded with this grid (NFR-04, NFR-06).
    pub fn metadata(&self) -> &GeoidMetadata {
        &self.metadata
    }

    /// Grid geometry: corner, spacing and dimensions.
    pub fn geometry(&self) -> GeoidGeometry {
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
        let index = row
            .checked_mul(self.geometry.cols)
            .and_then(|offset| offset.checked_add(col))
            .ok_or_else(|| malformed("sample index overflow"))?;
        self.undulations_m
            .get(index)
            .copied()
            .ok_or_else(|| malformed("sample index outside the grid"))
    }

    /// Fractional row and column of a covered coordinate.
    fn grid_position(&self, position: LatLon) -> Result<(f64, f64), GeodesyError> {
        crate::ellipsoid::validate(position)?;
        let row = (position.lat_deg.0 - self.geometry.lat_min_deg) / self.geometry.lat_step_deg;
        let col = (position.lon_deg.0 - self.geometry.lon_min_deg) / self.geometry.lon_step_deg;
        let last_row = (self.geometry.rows - 1) as f64;
        let last_col = (self.geometry.cols - 1) as f64;
        if row < -EDGE_TOLERANCE_CELLS
            || col < -EDGE_TOLERANCE_CELLS
            || row > last_row + EDGE_TOLERANCE_CELLS
            || col > last_col + EDGE_TOLERANCE_CELLS
        {
            return Err(GeodesyError::NoCoverage {
                lat_deg: position.lat_deg.0,
                lon_deg: position.lon_deg.0,
            });
        }
        Ok((row.clamp(0.0, last_row), col.clamp(0.0, last_col)))
    }
}

impl GeoidModel for GeoidGrid {
    /// Bilinear undulation at a coordinate, in metres (SDD section 6.1).
    ///
    /// Cells are interpolated linearly in latitude and longitude. The grid
    /// edge is inclusive within floating-point noise; anything beyond it
    /// returns [`GeodesyError::NoCoverage`].
    fn undulation_m(&self, position: LatLon) -> Result<f64, GeodesyError> {
        let (row, col) = self.grid_position(position)?;
        let row0 = (row.floor() as usize).min(self.geometry.rows - 2);
        let col0 = (col.floor() as usize).min(self.geometry.cols - 2);
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

/// Validate the corner, spacing and dimensions of a grid.
fn validate_geometry(geometry: GeoidGeometry) -> Result<(), GeodesyError> {
    if geometry.rows < 2 || geometry.cols < 2 {
        return Err(malformed("grid needs at least two rows and columns"));
    }
    if !geometry.lat_min_deg.is_finite() || !geometry.lon_min_deg.is_finite() {
        return Err(GeodesyError::NotFinite);
    }
    if !geometry.lat_step_deg.is_finite() || geometry.lat_step_deg <= 0.0 {
        return Err(malformed("latitude step must be positive and finite"));
    }
    if !geometry.lon_step_deg.is_finite() || geometry.lon_step_deg <= 0.0 {
        return Err(malformed("longitude step must be positive and finite"));
    }

    let lat_max_deg = geometry.lat_min_deg + (geometry.rows - 1) as f64 * geometry.lat_step_deg;
    let lon_max_deg = geometry.lon_min_deg + (geometry.cols - 1) as f64 * geometry.lon_step_deg;
    if geometry.lat_min_deg < -90.0 - EDGE_TOLERANCE_DEG || lat_max_deg > 90.0 + EDGE_TOLERANCE_DEG
    {
        return Err(malformed("latitude span leaves [-90, 90] degrees"));
    }
    if geometry.lon_min_deg < -180.0 - EDGE_TOLERANCE_DEG
        || lon_max_deg > 180.0 + EDGE_TOLERANCE_DEG
    {
        return Err(malformed("longitude span leaves [-180, 180] degrees"));
    }
    Ok(())
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

/// Parse one finite `f64` field.
fn parse_number(raw: &str, field: &str, line_number: usize) -> Result<f64, GeodesyError> {
    let value = raw.parse::<f64>().map_err(|_| {
        malformed(format!(
            "line {line_number}: field `{field}` is not a number: `{raw}`"
        ))
    })?;
    if !value.is_finite() {
        return Err(malformed(format!(
            "line {line_number}: field `{field}` must be finite"
        )));
    }
    Ok(value)
}

/// Parse one positive `usize` field.
fn parse_count(raw: &str, field: &str, line_number: usize) -> Result<usize, GeodesyError> {
    let value = raw.parse::<usize>().map_err(|_| {
        malformed(format!(
            "line {line_number}: field `{field}` is not a count: `{raw}`"
        ))
    })?;
    if value == 0 {
        return Err(malformed(format!(
            "line {line_number}: field `{field}` must be at least one"
        )));
    }
    Ok(value)
}

/// Reject an empty metadata value.
fn require_non_empty(raw: &str, field: &str, line_number: usize) -> Result<String, GeodesyError> {
    if raw.is_empty() {
        return Err(malformed(format!(
            "line {line_number}: field `{field}` is empty"
        )));
    }
    Ok(raw.to_owned())
}

/// Require a header field to have been present.
fn require<T>(value: Option<T>, field: &str) -> Result<T, GeodesyError> {
    value.ok_or_else(|| malformed(format!("missing field `{field}`")))
}
