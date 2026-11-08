# Third-party inventory

NFR-06 requires every dependency, copied snippet and dataset to be recorded
with its licence before it enters the build. This file is the working
inventory. `deny.toml` enforces the dependency allow-list in CI.

## Project licence

The project is **AGPL-3.0-only**. The root `LICENSE` is accepted. AGPL is the
outbound licence, not a dependency licence, so it is not in the `allow` list.

## Rust crates

Versions are resolved in `Cargo.lock`. A crate not listed here must be added
with its licence and purpose before it is used. "Verify at lock" means the
licence is expected to be compatible but must be checked against the exact
locked version before release.

| Crate | Licence | Purpose | Status |
| --- | --- | --- | --- |
| `anyhow` | MIT OR Apache-2.0 | Top-level binary errors only | In scaffold |
| `axum` | MIT | HTTP edge | In scaffold |
| `clap` | MIT OR Apache-2.0 | CLI parsing | In scaffold |
| `serde` | MIT OR Apache-2.0 | Serialization | In scaffold |
| `serde_json` | MIT OR Apache-2.0 | JSON manifests and API payloads | Workspace |
| `thiserror` | MIT OR Apache-2.0 | Typed library errors | In scaffold |
| `tokio` | MIT | Async edge and worker runtime | In scaffold |
| `tokio-util` | MIT | `CancellationToken` and task utilities | In scaffold |
| `tracing` | MIT | Structured logging | In scaffold |
| `tracing-subscriber` | MIT | Log formatting and filtering | In scaffold |
| `uuid` | Apache-2.0 OR MIT | UUIDv7 identifiers | In scaffold |
| `rayon` | MIT OR Apache-2.0 | Fixed-chunk parallel reductions | Approved, later |
| `nalgebra` | Apache-2.0 | Linear algebra and covariance | In `sim` (0.35) for the Fisher matrix and in `events` for the fit normal equations |
| `realfft` | MIT | Real FFT for the STFT | In `dsp` (3.5.1) |
| `rustfft` | MIT OR Apache-2.0 | FFT engine behind `realfft` | Indirect, via `realfft` (6.4.1) |
| `arrow` | Apache-2.0 | Columnar track store | Approved, later |
| `parquet` | Apache-2.0 | Track file format | Approved, later |
| `sqlx` | MIT OR Apache-2.0 | Relational repositories | Approved, later |
| `reqwest` | MIT OR Apache-2.0 | ADS-B archive client | In `adsb` (0.12) with blocking and `rustls-tls-native-roots` |
| `rustls` | Apache-2.0 OR MIT OR ISC | TLS for `reqwest` | Indirect, via `reqwest` (0.12) |
| `levenberg-marquardt` | MIT | Nonlinear least squares, with an in-house fallback | Approved, verify at lock |
| `statrs` | MIT | Chi-square and distribution functions | Approved, later |
| `tempfile` | MIT OR Apache-2.0 | Temporary test and scratch directories | Dev-dependency of `sim` and `dsp` |
| `proptest` | MIT OR Apache-2.0 | Property tests | Approved, later |
| `rand` | MIT OR Apache-2.0 | Seeded simulator randomness | In `sim` (0.9) |
| `rand_chacha` | MIT OR Apache-2.0 | Seeded ChaCha20 generator for reproducible noise and feeds | In `sim` (0.9) |
| `hound` | Apache-2.0 | 16-bit PCM WAV writing for simulator output | In `sim` (3.5) |
| `toml` | MIT OR Apache-2.0 | `config/default.toml` | Needed, not added |
| `sha2` | MIT OR Apache-2.0 | Manifest SHA-256 | Workspace |
| `hmac` | MIT OR Apache-2.0 | Keyed IP hashing | Needed, not added |
| `subtle` | BSD-3-Clause | Constant-time token comparison | Needed, not added |
| `flate2` | MIT OR Apache-2.0 | Compressed ADS-B dumps, if needed | Conditional |
| `image` | MIT OR Apache-2.0 | Spectrogram and map figures | Candidate, verify at lock |
| `plotters` | MIT | Figures, if `image` is not enough | Candidate, verify at lock |
| `metrics` | MIT | Prometheus counters for FR-OP-03 | Candidate, verify at lock |
| `tower` | MIT | Axum middleware and limits | Approved, later |

- `realfft` also pulls `primal-check`, `strength_reduce` and `transpose` (all
  MIT OR Apache-2.0); `num-complex` and `num-traits` were already in the lock
  for `nalgebra`. `cargo deny check` covers them.

### Excluded or conditional

- `webpki-roots` is MPL-2.0 and is not allowed. Use
  `rustls-tls-native-roots` or add a scoped exception in `deny.toml` after
  review (Q-4).
- No crate with a GPL, LGPL or AGPL dependency licence is permitted without a
  recorded owner decision. This does not apply to the project's own AGPL
  licence.

## Subprocesses and media tools

| Tool | Licence note | Use |
| --- | --- | --- |
| FFmpeg | Depends on build flags; LGPL-2.1-or-later for a default build, GPL for some codecs | Decode untrusted media in a restricted subprocess (SDD T-1). Record `ffmpeg -version` and the build configuration in the run manifest. Do not link FFmpeg libraries into the Rust binaries. |

## Data sources

Every dataset must record its version or snapshot date, licence, attribution
and redistribution policy in the run manifest (NFR-04, NFR-06).

| Source | Expected licence or terms | Use | Status |
| --- | --- | --- | --- |
| OpenSky Network | Open academic use, attribution; verify current terms for redistribution | ADS-B tracks | OpenSky states client in `adsb`; fixture `crates/adsb/tests/fixtures/opensky_states.json`; terms still to confirm for redistribution |
| adsb.lol | Open data; verify the current ODbL or equivalent terms | ADS-B tracks | Planned, terms must be confirmed |
| SRTM or Copernicus DEM | Public domain or open with attribution; verify the chosen product | Terrain | SRTMGL1 crop in `crates/geodesy/tests/fixtures/srtm_everest_33x33.txt`, extracted 2026-11-07 from AWS Open Data `N27E086.hgt.gz` (SHA-256 `0e95b43508321ee2b3d9454797130bf1c972737377eff4cb07b4cf912138aad1`); SRTM is a public-domain NASA/USGS product; format in `docs/dem-grid-format.md` |
| EGM96 / EGM2008 | Public domain geoid grid (NGA) | Geoid correction | EGM96 15-minute subset in `crates/geodesy/tests/fixtures/egm96_uk_3deg.txt`, extracted 2026-11-07 from PROJ `egm96_15.gtx` (SHA-256 `c02a6eb70a7a78efebe5adf3ade626eb75390e170bb8b3f36136a2c28f5326a0`); format in `docs/geoid-grid-format.md` |
| Natural Earth | Public domain | Land mask and map background | Planned |
| Dove's Guide for Church Bell Ringers | Reference only; do not copy the dataset without permission | Grid-hum context | Reference only |
| Author's own recordings | Written consent from the author | Real-clip calibration | Planned |
| Phone-video samples | Contributor consent and anonymized location | Codec and detector tests | Planned |

`crates/geodesy/tests/fixtures/egm96_uk_3deg.txt` and
`crates/geodesy/tests/fixtures/srtm_everest_33x33.txt` are the only datasets
committed so far. Both are test-only. `docs/geoid-grid-format.md` and
`docs/dem-grid-format.md` record their extraction, format, source hashes and
licences.

## Copied code and reference repositories

- **None.** No code has been copied into this repository.
- `Local/franken_doppler_finder` is a functional reference only. It is
  git-ignored, its licence is not inherited, and no source from it may be
  pasted here. Any future reuse requires a proposal, a licence check and an
  entry in this file before the code lands (PRD D-2, Norms section 1).
- The architecture diagram in `docs/assets/architecture.png` is part of this
  project's documentation. Record its source if it was generated elsewhere.

## Review rule

A pull request that adds a dependency, dataset, copied snippet or subprocess
updates this file in the same commit. CI fails if `cargo deny check` finds a
licence outside `deny.toml`.
