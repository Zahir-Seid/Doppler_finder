# Geoid grid format

`geodesy::GeoidGrid` reads the `doppler-geoid` version 1 text format. It exists
so FR-DA-02 can be implemented without a new crate or a binary geoid decoder;
the loader is deliberately small and every grid carries its provenance for the
run manifest (NFR-04).

## File layout

The file is UTF-8 text. Blank lines and lines starting with `#` are comments.
Header fields are `key=value`, one per line, and may appear in any order. A
line containing only `undulation_m` ends the header. Every following token is
one sample in metres.

```text
# optional comments
name=EGM96
version=15-minute grid (1996)
source=PROJ egm96_15.gtx, NGA EGM96
licence=Public domain (NGA)
lat_min_deg=50.0
lon_min_deg=-2.0
lat_step_deg=0.25
lon_step_deg=0.25
rows=13
cols=13
undulation_m
48.049 47.475 46.953 ...
...
```

Required header fields: `name`, `version`, `source`, `licence`,
`lat_min_deg`, `lon_min_deg`, `lat_step_deg`, `lon_step_deg`, `rows` and
`cols`. All four metadata strings must be non-empty. `rows` and `cols` are at
least two and the sample count must equal their product.

## Grid convention

- Row zero is the **southern** edge and column zero is the **western** edge.
- Samples are row-major: all longitude samples of the southernmost row first.
- `lat_step_deg` and `lon_step_deg` are positive degrees; latitude is
  geodetic latitude on WGS84.
- Undulations are metres above the WGS84 ellipsoid, positive where the geoid
  lies above the ellipsoid.
- A grid must stay inside `[-90, 90]` latitude and `[-180, 180]` longitude.
  It does not wrap across the antimeridian; split such a model into grids that
  do not cross it.
- Sampling is bilinear. The edge is inclusive within `1e-9` of a cell;
  anything beyond returns `GeodesyError::NoCoverage`.

## Height conversion

SDD section 6.1 defines the relation between orthometric height `H` (above the
geoid, the datum public DEMs publish) and ellipsoidal height `h` (above the
WGS84 ellipsoid, which the propagation maths needs):

```text
h = H + N
H = h - N
```

`N` is `GeoidModel::undulation_m`. Use `orthometric_to_ellipsoidal` and
`ellipsoidal_to_orthometric` rather than adding the undulation by hand.

## Manifest and provenance

`GeoidMetadata::model_label()` returns `"<name> <version>"` for
`ModelVersions::model`. Put `source` and `licence` in the run's data-source
record so the exact grid can be identified and its terms checked (NFR-04,
NFR-06).

## Test fixture

`crates/geodesy/tests/fixtures/egm96_uk_3deg.txt` is a 13 by 13 subset of the
NGA EGM96 15-minute geoid over 50–53 N, 2 W–1 E. It was extracted from PROJ's
`egm96_15.gtx` on 2026-11-07:

- source: <https://download.osgeo.org/proj/vdatum/egm96_15/egm96_15.gtx>
- source SHA-256: `c02a6eb70a7a78efebe5adf3ade626eb75390e170bb8b3f36136a2c28f5326a0`
- licence: NGA EGM96 is a public-domain geodetic product.

The fixture is test data only. Production use should load the full grid from a
cached product and record its own version and hash.
