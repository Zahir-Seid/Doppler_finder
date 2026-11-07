//! Shared regular-grid geometry and header-field parsing.
//!
//! Geoid and terrain grids use the same row-major convention, the same
//! inclusive edges and the same bilinear cell lookup. Keeping the geometry and
//! the text-field helpers here stops the two datums from drifting apart.

use core_types::LatLon;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::GeodesyError;

/// Fraction of a grid cell a coordinate may fall outside the last sample and
/// still count as covered. It absorbs floating-point noise at a grid edge.
pub(crate) const EDGE_TOLERANCE_CELLS: f64 = 1.0e-9;

/// Angular slack when validating a grid's north or east edge, in degrees.
pub(crate) const EDGE_TOLERANCE_DEG: f64 = 1.0e-9;

/// Geometry of a regular latitude/longitude grid.
///
/// `lat_min_deg` and `lon_min_deg` are the southern and western edges; the
/// steps are positive degrees between neighbouring samples. Row zero is the
/// southern edge and column zero is the western edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridGeometry {
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

impl GridGeometry {
    /// Check the corner, spacing and dimensions, returning a reason on failure.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.rows < 2 || self.cols < 2 {
            return Err("grid needs at least two rows and columns".to_owned());
        }
        if !self.lat_min_deg.is_finite() || !self.lon_min_deg.is_finite() {
            return Err("grid corner must be finite".to_owned());
        }
        if !self.lat_step_deg.is_finite() || self.lat_step_deg <= 0.0 {
            return Err("latitude step must be positive and finite".to_owned());
        }
        if !self.lon_step_deg.is_finite() || self.lon_step_deg <= 0.0 {
            return Err("longitude step must be positive and finite".to_owned());
        }

        let lat_max_deg = self.lat_min_deg + (self.rows - 1) as f64 * self.lat_step_deg;
        let lon_max_deg = self.lon_min_deg + (self.cols - 1) as f64 * self.lon_step_deg;
        if self.lat_min_deg < -90.0 - EDGE_TOLERANCE_DEG || lat_max_deg > 90.0 + EDGE_TOLERANCE_DEG
        {
            return Err("latitude span leaves [-90, 90] degrees".to_owned());
        }
        if self.lon_min_deg < -180.0 - EDGE_TOLERANCE_DEG
            || lon_max_deg > 180.0 + EDGE_TOLERANCE_DEG
        {
            return Err("longitude span leaves [-180, 180] degrees".to_owned());
        }
        Ok(())
    }

    /// Fractional row and column of a covered coordinate.
    pub(crate) fn grid_position(&self, position: LatLon) -> Result<(f64, f64), GeodesyError> {
        crate::ellipsoid::validate(position)?;
        let row = (position.lat_deg.0 - self.lat_min_deg) / self.lat_step_deg;
        let col = (position.lon_deg.0 - self.lon_min_deg) / self.lon_step_deg;
        let last_row = (self.rows - 1) as f64;
        let last_col = (self.cols - 1) as f64;
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

    /// Row-major index of a sample, if the indices are inside the grid.
    pub(crate) fn sample_index(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        row.checked_mul(self.cols)
            .and_then(|offset| offset.checked_add(col))
    }

    /// Index of the southern row of the cell containing a fractional row.
    pub(crate) fn lower_row(&self, row: f64) -> usize {
        (row.floor() as usize).min(self.rows - 2)
    }

    /// Index of the western column of the cell containing a fractional column.
    pub(crate) fn lower_col(&self, col: f64) -> usize {
        (col.floor() as usize).min(self.cols - 2)
    }
}

/// Parse one finite `f64` field, returning a reason on failure.
pub(crate) fn parse_number(raw: &str, field: &str, line_number: usize) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|_| format!("line {line_number}: field `{field}` is not a number: `{raw}`"))?;
    if !value.is_finite() {
        return Err(format!(
            "line {line_number}: field `{field}` must be finite"
        ));
    }
    Ok(value)
}

/// Parse one positive `usize` field, returning a reason on failure.
pub(crate) fn parse_count(raw: &str, field: &str, line_number: usize) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("line {line_number}: field `{field}` is not a count: `{raw}`"))?;
    if value == 0 {
        return Err(format!(
            "line {line_number}: field `{field}` must be at least one"
        ));
    }
    Ok(value)
}

/// Reject an empty metadata value.
pub(crate) fn require_non_empty(
    raw: &str,
    field: &str,
    line_number: usize,
) -> Result<String, String> {
    if raw.is_empty() {
        return Err(format!("line {line_number}: field `{field}` is empty"));
    }
    Ok(raw.to_owned())
}

/// A parsed `key=value` header and the raw sample tokens after its section.
pub(crate) type Header = BTreeMap<String, (String, usize)>;

/// Split a grid text file into header fields and sample tokens.
///
/// Blank lines and `#` comments are ignored. The header ends at the line that
/// contains only `section`; every later token is returned with its line
/// number. `allowed` lists the accepted header keys, so an unknown or duplicate
/// key is a format error rather than a silently ignored typo.
pub(crate) fn split_header(
    text: &str,
    section: &str,
    allowed: &[&str],
) -> Result<(Header, Vec<(String, usize)>), String> {
    let mut fields = Header::new();
    let mut samples = Vec::new();
    let mut in_samples = false;

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if in_samples {
            for token in line.split_whitespace() {
                samples.push((token.to_owned(), line_number));
            }
            continue;
        }
        if line == section {
            in_samples = true;
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            return Err(format!(
                "line {line_number}: expected `key=value` or `{section}`"
            ));
        };
        let key = raw_key.trim();
        if !allowed.contains(&key) {
            return Err(format!("line {line_number}: unknown field `{key}`"));
        }
        if fields
            .insert(key.to_owned(), (raw_value.trim().to_owned(), line_number))
            .is_some()
        {
            return Err(format!("line {line_number}: duplicate field `{key}`"));
        }
    }

    if !in_samples {
        return Err(format!("missing `{section}` sample section"));
    }
    Ok((fields, samples))
}

/// Fetch a required header value and the line it appeared on.
pub(crate) fn field<'a>(fields: &'a Header, name: &str) -> Result<(&'a str, usize), String> {
    fields
        .get(name)
        .map(|(value, line)| (value.as_str(), *line))
        .ok_or_else(|| format!("missing field `{name}`"))
}

/// Fetch a required non-empty header string.
pub(crate) fn field_text(fields: &Header, name: &str) -> Result<String, String> {
    let (value, line) = field(fields, name)?;
    require_non_empty(value, name, line)
}

/// Fetch a required finite header number.
pub(crate) fn field_number(fields: &Header, name: &str) -> Result<f64, String> {
    let (value, line) = field(fields, name)?;
    parse_number(value, name, line)
}

/// Fetch a required positive header count.
pub(crate) fn field_count(fields: &Header, name: &str) -> Result<usize, String> {
    let (value, line) = field(fields, name)?;
    parse_count(value, name, line)
}
