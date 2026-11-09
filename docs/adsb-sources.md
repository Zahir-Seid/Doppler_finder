# ADS-B archive sources

FR-DA-01 ingests open ADS-B tracks for a region and date. ADSB-01 adds the
guarded OpenSky states client and the sampling statistics that feed the run
manifest. No test opens a socket: the suite uses a hand-built payload that
follows the documented OpenSky response schema.

## OpenSky states client

`adsb::OpenSkyClient` requests:

```text
GET https://opensky-network.org/api/states/all
    ?lamin=<south>&lomin=<west>&lamax=<north>&lomax=<east>&time=<unix-seconds>
```

The bounding box is in decimal degrees and `time` is the window start in Unix
seconds. OpenSky returns one snapshot per request, so samples outside the
requested window are dropped after parsing.

Authentication uses an OAuth2 bearer token from the caller. Token acquisition
and refresh are outside the client; `None` uses anonymous access. Do not log a
request's token: `HttpRequest`'s `Debug` output only records whether one is
present.

OpenSky's REST states endpoint serves only recent state vectors (anonymous
access is limited to the latest snapshot; authenticated access reaches about
one hour back). Full-day history uses the adsb.lol daily dumps below; the
OpenSky Trino interface remains an alternative not yet implemented.

## adsb.lol daily dumps

`adsb::dump` ingests the adsb.lol `globe_history_{year}` releases: one GitHub
release per UTC day and feed (`prod`, `mlatonly`, `staging`), tagged
`v{YYYY.MM.DD}-planes-readsb-{feed}-{N}`, with large releases split into
`.tar.aa`, `.tar.ab`, … parts that concatenate into one tar stream. The tar
holds `trace_full_*.json` members — one per aircraft — each gzipped by magic
(`1f 8b`) despite the `.json` name.

Each member is a readsb trace:

```text
{"icao": ..., "r": ..., "t": <type>, "timestamp": <unix s>, "trace": [row, ...]}
```

A row is `[dt, lat, lon, alt_baro|"ground"|null, gs_kt, track, flags,
vrate_fpm, details|null, source, alt_geom_ft, geom_rate_fpm, ias, roll]` with
`dt` in seconds after `timestamp`; fields from index 9 exist only in files
from 2022 on. Row flags: 1 stale, 2 new leg, 4 geometric vertical rate,
8 geometric altitude. Units convert to the track model (feet→m, knots→m/s,
ft/min→m/s); `geometric_altitude_m` is ellipsoidal so `alt_geom_ft` needs no
geoid correction.

`ingest_reader`/`ingest_path` keep rows inside the query window and region —
plus segment-bracket anchors and two neighbours each side so interpolation
stays anchored — and return `FlightTrack`s plus `DumpStats`. `AdsbLolArchive`
implements `TrackSource` over a local tar, directory of `trace_full_*` files
or single member for offline replay; release discovery and download are
fetch-layer work. The parser is ported from `franken_doppler_finder`
`fdop-data/src/ingest.rs` (see `docs/third_party.md`), single-threaded, and
keeps every aircraft — engine classification happens in `adsb::legs` at read
time. readsb reports no staleness gap, so `latency_s` stays `None`; the
stale/new-leg/source bits land in `TrackSample::flags`
(`FLAG_STALE`, `FLAG_NEW_LEG`, `FLAG_NEW_LEG_AFTER`, `FLAG_MLAT`,
`FLAG_TISB`). Baro-only rows reuse the barometric altitude for
`geometric_altitude_m` under `FLAG_GEOM_FROM_BARO`.

## Request bounds

Every request passes through `HttpPolicy` before it leaves the process:

- **HTTPS only**, default port 443, no embedded credentials and no URL
  fragment.
- **Host allow-list.** The URL host must match an entry exactly. The OpenSky
  client uses `opensky-network.org`; `OPENSKY_HOST` names it.
- **No redirects.** The production `ReqwestTransport` refuses redirects so an
  allow-listed host cannot forward the request to an unlisted one.
- **Timeout.** Applied by the blocking `reqwest` client with
  `rustls-tls-native-roots`.
- **Size cap.** The transport stops reading at the cap plus one byte and
  returns `AdsbError::ResponseTooLarge`; an oversized body is never fully
  buffered.
- **Bounded retries.** Statuses 408, 425, 429, 500, 502, 503 and 504, and
  transport errors, are retried up to the configured attempt count. Other
  non-success statuses fail immediately. The size-cap error is not retried.

A typical policy:

```rust
let policy = HttpPolicy::new(
    vec!["opensky-network.org".to_owned()],
    Duration::from_secs(20),
    8 * 1024 * 1024,
    3,
    Duration::from_secs(1),
)?;
let transport = policy.reqwest_transport()?;
let client = OpenSkyClient::new(transport, policy, token);
```

## From state vectors to tracks

`parse_opensky_states` groups rows by ICAO24 and sorts samples by time. It skips
a row when it has no timestamp, position, geometric altitude, or when it is a
surface report (`on_ground`). It also drops samples outside the query window.
Malformed rows, non-finite values, coordinates outside WGS84 bounds and bad
ICAO24 strings are parse errors rather than silently discarded data.

The track model keeps geometric altitude because that is the datum the
propagation maths needs, and it also keeps barometric altitude, vertical rate
and the source's position-quality flags. OpenSky field 7 becomes
`barometric_altitude_m`, field 11 becomes `vertical_rate_m_per_s`, and a
`position_source` of 2 sets `FLAG_MLAT`. Both altitude datums are stored by the
Parquet cache so the error budget can treat the datum as a source of error.

## Sampling statistics

`sampling_stats` computes the median and nearest-rank 90th percentile of every
positive gap between consecutive samples across a batch. Duplicate timestamps
are ignored because they do not describe the source's sampling interval.
`SamplingStats::data_source` builds the `core_types::DataSource` manifest
record, so a result is tied to the measured intervals that produced it
(FR-DA-01, NFR-04).

Source latency is recorded as reported position staleness. OpenSky state
vectors carry `time_position` and `last_contact`; `TrackSample::latency_s`
stores their difference in seconds, and `sampling_stats` reports the median
and nearest-rank 90th percentile into `DataSource::latency_median_s` /
`latency_p90_s` in the run manifest (FR-DA-01). Sources that do not report a
staleness pair leave the fields empty rather than guessing. The Parquet store
keeps the per-sample value in the `latency_s` column (schema v2) and
`IngestRun` recomputes the same percentiles per source and day on replay.
Archive-publication latency — how long after the fact a dump or API response
became available — is a different quantity and is still not measured; it
belongs with the full-day ingest path.

## Test fixture

`crates/adsb/tests/fixtures/opensky_states.json` is a hand-built response that
follows the OpenSky state-vector field order. It contains two aircraft with
time-ordered samples, one state without geometric altitude, one surface report
and one sample beyond the test window. It contains no real flight data and
needs no attribution.

Tests: `crates/adsb/tests/da_01.rs`, covering parsing, window filtering,
malformed payloads, the empty `states` case, URL allow-listing, the size cap,
bounded retries, non-retryable statuses, request construction, query validation
and sampling statistics.
