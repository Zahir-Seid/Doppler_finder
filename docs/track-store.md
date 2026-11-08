# Track store and offline replay

FR-DA-04 requires cached ingest products so a rerun works without the network.
The `adsb` crate provides `TrackStore`, a directory-backed Parquet cache that
keeps the archive's original samples, partitions them for pruning, and replays
them offline. SDD sections 9.1 and 9.2 define the store and its schema.

## Layout

```text
<root>/source=<kind>/date=YYYY-MM-DD/lat=<floor>/lon=<floor>/part-<sha256>.parquet
```

- `source` is `opensky` or `adsblol`.
- `date` is the UTC day of the sample's own `ts_ms`.
- `lat` and `lon` are one-degree tile floors, clamped so a pole maps to
  `lat=89` or `lat=-90` and the antimeridian maps to `lon=179` or `lon=-180`.
- The file name is the SHA-256 of the file bytes. Identical input therefore
  produces the identical path and identical bytes; a rewrite is idempotent.

A track that crosses midnight or a tile boundary is split across partitions.
Replay groups it back by source and ICAO24, so partitioning never changes the
track's samples.

## Schema

One row per sample, following SDD 9.2:

| Column | Arrow type | Notes |
| --- | --- | --- |
| `icao24` | UTF-8 | Lowercase hexadecimal transponder address |
| `ts_ms` | Int64 | The source's own UTC timestamp in milliseconds |
| `lat_deg`, `lon_deg` | Float64 | WGS84 |
| `alt_geom_m` | Float32, nullable | Geometric altitude, kept as reported |
| `alt_baro_m` | Float32, nullable | Barometric altitude, kept as reported |
| `gs_mps` | Float32, nullable | Ground speed |
| `trk_deg` | Float32, nullable | Track angle clockwise from north |
| `vrate_mps` | Float32, nullable | Vertical rate, positive climbing |
| `type_code` | UTF-8, nullable | ICAO aircraft type |
| `source` | UTF-8 | `opensky` or `adsblol` |
| `flags` | UInt8 | Quality flags such as MLAT or interpolated |
| `callsign` | UTF-8, nullable | Extra column so replay returns complete tracks |
| `latency_s` | Float32, nullable | Reported position staleness in seconds (schema v2) |

SDD 9.2 specifies float32 for the altitude, speed, track and rate columns; the
store follows that, so those values round-trip within float32 precision.
Latitude, longitude and the timestamp are float64/int64 and round-trip exactly.
`latency_s` was added in schema version 2; the reader treats a missing column
as "no staleness reported", so v1 files still replay.
Files use SNAPPY compression and carry `doppler.schema_version`,
`doppler.source` and `doppler.date` in Parquet key-value metadata.

Original sampling is never resampled. Interpolation is a separate read-side
operation (`adsb::interpolate`), and its results are marked
`FLAG_INTERPOLATED` so a cached row is never mistaken for an archive sample.

## Writing

```rust
let store = TrackStore::new(cache_root);
let runs = store.write_tracks(&tracks)?;
```

`write_tracks` validates every track, groups samples by source, date and tile,
sorts rows by timestamp and ICAO24, and writes one file per group. It returns
one `IngestRun` per source and UTC day with:

- the row count,
- the median and nearest-rank 90th-percentile gap measured from the stored
  timestamps,
- the file path, SHA-256 and row count for each partition,
- a combined SHA-256 over the sorted file paths and digests.

## Offline replay

```rust
let tracks = store.read_tracks(&RegionQuery {
    min_lat_deg: 8.0,
    min_lon_deg: 38.0,
    max_lat_deg: 10.0,
    max_lon_deg: 40.0,
    from: UtcMillis(start_ms),
    to: UtcMillis(end_ms),
})?;
```

Replay never touches the network. Directory names prune by date and
one-degree tile before a file is opened; rows inside each surviving file are
filtered again by the exact inclusive time window and region. Samples keep
their original timestamps and order. Tracks are returned ordered by source and
ICAO24 for deterministic downstream results.

`read_tracks` returns an empty vector when the root or the requested partitions
do not exist. Coverage policy belongs to ADSB-04, not to the cache.

## Ingest statistics

`store.ingest_runs()` recomputes statistics by reading the stored files rather
than trusting a sidecar record. If a partition is replaced or deleted, the
returned median, p90, row counts and digests describe what is actually on
disk. `IngestRun.file_sha256` feeds the run manifest's data-source record
(FR-DA-01, NFR-04).

## Tests

`crates/adsb/tests/da_04.rs` covers round-trip of both altitude datums, speed,
track, vertical rate and flags; partition paths for source, date and tile;
region and window pruning with inclusive edges; exact timestamp preservation
with no resampling; per-source/day ingest statistics; deterministic writes;
invalid queries and tracks; empty input and a missing root; and flag
round-trips.
