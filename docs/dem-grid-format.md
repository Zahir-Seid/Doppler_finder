# DEM grid format

`geodesy::DemGrid` samples terrain heights so FR-DA-02 can place the observer
and a propagation path above real ground. It reads the `doppler-dem` version 1
text format and decompressed SRTM `.hgt` tiles without a new crate or a
binary-dataset dependency.

## File layout

The text file is UTF-8. Blank lines and lines starting with `#` are comments.
Header fields are `key=value`, one per line, and may appear in any order. A
line containing only `elevation_m` ends the header. Every following token is
one elevation in metres above the geoid; the token `void` marks a missing
sample.

```text
# optional comments
product=SRTM
version=SRTMGL1 v3 (1 arc-second)
source=AWS Open Data N27E086, NASA/USGS SRTM
licence=Public domain (NASA/USGS)
vertical_error_m=16.0
lat_min_deg=27.983611111111109
lon_min_deg=86.920555555555552
lat_step_deg=0.000277777777777778
lon_step_deg=0.000277777777777778
rows=33
cols=33
elevation_m
7874 7888 7905 ...
...
```

Required header fields: `product`, `version`, `source`, `licence`,
`vertical_error_m`, `lat_min_deg`, `lon_min_deg`, `lat_step_deg`,
`lon_step_deg`, `rows` and `cols`. All four metadata strings must be non-empty.
`vertical_error_m` is the published or measured one-sigma vertical error in
metres and must be finite and non-negative (SDD section 7). `rows` and `cols`
are at least two and the sample count must equal their product.

## Grid convention

- Row zero is the **southern** edge and column zero is the **western** edge.
- Samples are row-major: all longitude samples of the southernmost row first.
- `lat_step_deg` and `lon_step_deg` are positive degrees; latitude is
  geodetic latitude on WGS84.
- Elevations are metres **above the geoid** (orthometric height `H`), the
  datum public DEMs publish.
- A grid must stay inside `[-90, 90]` latitude and `[-180, 180]` longitude.
  It does not wrap across the antimeridian.
- Sampling is bilinear. The edge is inclusive within `1e-9` of a cell;
  anything beyond returns `GeodesyError::NoCoverage`.
- A cell with any void corner returns `NoCoverage`, so a missing sample is
  never silently replaced by zero.

## Observer height

SDD section 6.1 builds the observer's ellipsoidal height from terrain, the
nominal ear height and the geoid undulation:

```text
h = H_terrain + H_ear + N
```

`observer_height_m(terrain, geoid, position, ear_height)` applies this exactly
once. Do not feed an orthometric terrain height straight into the ellipsoid,
and do not add the undulation again after calling the helper. A negative or
non-finite ear height returns `GeodesyError::InvalidEarHeight`.

## SRTM `.hgt` tiles

`DemGrid::from_srtm_hgt` reads a decompressed `.hgt` payload. SRTM stores signed
16-bit **big-endian** samples, one degree square, first row at the north edge
and first column at the west edge. `SrtmTile::from_name` parses the south-west
corner from a name such as `N27E086`; one more sample per side is present than
the `samples_per_degree` value because both edges are included. The loader:

- checks that the byte length is `2 * (samples_per_degree + 1)^2`;
- flips the north-first rows into the south-first grid convention;
- maps the `-32768` void marker to `None`; and
- records the tile's product metadata and vertical error.

Production tiles are usually distributed gzip-compressed; callers decompress
first. The 1-arc-second product uses 3600 samples per degree (3601 per side),
the 3-arc-second product 1200.

## Manifest and provenance

`DemMetadata::model_label()` returns `"<product> <version>"` for
`ModelVersions::model`. Put `source` and `licence` in the run's data-source
record so the exact terrain product can be identified and its terms checked
(NFR-04, NFR-06). The recorded `vertical_error_m` belongs in the error budget
alongside the geoid and ADS-B contributions.

## Test fixture

`crates/geodesy/tests/fixtures/srtm_everest_33x33.txt` is a 33 by 33
one-arc-second crop of SRTMGL1 tile `N27E086` around Mount Everest. It was
extracted on 2026-11-07 from the AWS Open Data terrain tiles mirror:

- source: <https://s3.amazonaws.com/elevation-tiles-prod/skadi/N27/N27E086.hgt.gz>
- source gzip SHA-256:
  `0e95b43508321ee2b3d9454797130bf1c972737377eff4cb07b4cf912138aad1`
- crop: rows 27–59 and columns 3314–3346 of the 3601 by 3601 tile
- licence: SRTM is a public-domain NASA/USGS product.

The fixture is test data only. Production use should load the full tile from a
cached product and record its own version and hash.
