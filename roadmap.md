# Roadmap — Doppler Locatability Audit

Execution plan for the implementing agent(s). This file orders the work; it does
not redefine requirements. If this file disagrees with the PRD, SDD or working
norms, those documents win.

- **Last updated:** 2026-11-08
- **Last recorded commit:** `f0355bb` (`develop`)
- **Current milestone:** M4 (ADS-B, terrain and atmosphere); M3 closed
- **Release under construction:** v0 research core

Read first, in this order:

1. `docs/PRD.md` — what and why, requirements, gates.
2. `docs/SDD.md` — architecture, methods, data design.
3. `docs/rust-working-norms.md` — how code is written and when it is done.
4. `docs/traceability.csv` — the live requirement scorecard.
5. `docs/validation-plan.md` — how each gate is measured.
6. This roadmap — the order to build it in.

---

## 1. Status snapshot

### 1.1 Scorecard

`docs/traceability.csv` currently tracks 50 items:

| Status | Count |
| --- | ---: |
| Done | 10 |
| Partial | 7 |
| In progress | 0 |
| Scaffolded | 2 |
| Not started | 29 |
| Deferred to v3 | 2 |

For **v0** (24 tracked rows, including gates G1 and G2): **12 done, 3 partial,
0 in progress, 9 not started**. The done rows are `FR-SIM-01` (synthesis and
truth), `FR-SIM-03` (Cramér–Rao bound), `FR-DM-01` (detection), `FR-DM-02`
(track-then-fit estimation), `FR-DM-03` (emission-time model), `FR-DM-04`
(harmonics combined coherently in the fit), `FR-DM-05` (direct estimation on
faint events), `FR-DM-06` (quality score and gate), `FR-DA-01` (ADS-B ingest
with sampling and latency in the manifest), `FR-DA-02` (geoid and
DEM), `FR-DA-03` (measured atmosphere with flagged fallback) and
`FR-DA-04` (Parquet track store and offline replay). The v0 partial
rows are `FR-OUT-04`, `FR-SIM-02` and `FR-SIM-04`;
`NFR-04` (v0–v1) is also partial, and `FR-IN-01` is v1.
`FR-DM-01` closed with
EVT-01, `FR-DM-02`/`FR-DM-04` with EVT-02, `FR-DM-05` with EVT-03,
`FR-DM-06` with EVT-04 and `FR-DM-03` with EVT-05: the detector runs on the
whitened z-map, the joint fit recovers the pass with a scaled covariance and
profile intervals, the direct estimator recovers the curve from a faint map
with no ridge input, the quality gate drops weak events with recorded reasons,
and a regression pins the emission-time model against a delayed-copy control.
The M3 end-to-end exit test now drives a simulated pass from audio through
whitening, detection and direct fitting to a retained quality assessment.
GEO-01 adds the EGM96 geoid grid and height conversion; GEO-02 adds the DEM
sampler, SRTM `.hgt` loading and observer-height composition, closing
`FR-DA-02`. ADSB-01 adds the allow-listed OpenSky states client and measured
sampling statistics. Reported position latency (`latency_s` →
`DataSource::latency_*`) and full-day adsb.lol dump ingest (`adsb::dump`,
ported from the reference implementation) closed `FR-DA-01` with ADSB-05;
release discovery/download remains fetch-layer work. ACO-02 adds the
Open-Meteo fetch adapter producing the normalized weather payload, closing
`FR-DA-03`. ADSB-02 adds the
partitioned Parquet track store and
offline replay, closing `FR-DA-04`. ADSB-03 adds Hermite interpolation for
the light-time solver, derived legs with engine class and closest-approach
prediction. ADSB-04 adds region support flags from measured coverage,
closing T-4's flag half (the Addis measurement itself stays open).

### 1.2 Code state

| Crate | State | What exists |
| --- | --- | --- |
| `core-types` | Usable | Units, two-clock bridge, IDs, cancellation, manifests and SHA-256 |
| `geodesy` | Partial | WGS84, ECEF/ENU forward and inverse; EGM96 grid with bilinear sampling and orthometric/ellipsoidal conversion; DEM text and SRTM `.hgt` grids with void handling and observer-height composition |
| `acoustics` | Partial | Atmosphere, emission-time Doppler, retarded-time solver, weather adapter with path-projected wind and flagged fallback |
| `sim` | Partial | Synthesis, noise, absorption, WAV/truth export, degraded feed, CRLB, one golden scenario |
| `dsp` | Partial | Kaiser/Hann windows, real STFT with zero-padding and sub-bin frequency, per-bin whitening, ridge ladder, FFmpeg decoder seam; band-limit, rational resampling and DC removal |
| `events` | Partial | Detector over the z-map, joint track-then-fit estimator, direct power estimator and quality gate; `Event`, `EventFit`, `EventObservation`, `EventQuality` |
| `adsb` | Partial | `TrackSource`, track model; allow-listed OpenSky states client with bounded retries and size caps; sampling median and p90 statistics; partitioned Parquet track store with offline replay and per-source/day ingest statistics; Hermite interpolation, derived legs, closest-approach prediction and region support flags; adsb.lol dump ingest and the Open-Meteo weather fetch adapter |
| `search` | Types only | `LocalizationEstimate`, `Verdict`, thresholds |
| `pipeline` | Types only | Stages, manifest re-export, progress trait, budgets |
| `store` | Types only | Job records/repository trait, limits, retention |
| `runtime` | Partial | `Scope`, cancellation bridge, timeout wrapper |
| `api` | Stub | `/healthz`, `/v1/health`, `/v1/version` |
| `worker` | Stub | Heartbeat and graceful shutdown only |
| `cli` | Stub | Argument parsing; commands log “not implemented” |

Quality gates on the current tree: `cargo fmt --check`, `clippy -D warnings` and
the full test suite pass. 317 tests pass (37 acoustics, 29 core-types, 74 dsp
unit, 1 M2 exit, 29 events unit, 2 M3 exit, 30 geodesy, 61 adsb, 52 sim unit,
2 sim golden). `cargo deny` and `cargo audit`
run in CI. The toolchain stays on stable Rust 1.99.1 with no nightly features
(NFR-05).

### 1.3 What “done” means here

A task is done only when all of the following are true:

- [ ] The behavior is implemented in the crate named by the task.
- [ ] At least one test names the requirement ID it implements.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
- [ ] `cargo test --workspace --locked` passes.
- [ ] `cargo deny check` and `cargo audit` pass (or CI does; record it).
- [ ] Public items are documented; formulas cite the SDD section and units.
- [ ] No `unwrap`, `expect`, `panic!`, `todo!` or unchecked indexing in library code.
- [ ] No new dependency without an entry in `docs/third_party.md`.
- [ ] Results are deterministic for a fixed seed across thread counts.
- [ ] `docs/traceability.csv` is updated in the same change.
- [ ] Open ambiguities are recorded in `docs/questions.md`.
- [ ] This roadmap's task status is updated.

---

## 2. How agents must work

1. **Pick the first unblocked task** in milestone order. Do not start M8–M11
   before G1 and G2 pass; M12 waits for G1–G6. The PRD phase order is deliberate.
2. **Read the requirement and its SDD section before writing code.** Cite the
   FR ID in the commit, PR and test.
3. **One task per branch and PR.** Prefer under 400 changed lines. Conventional
   Commits, for example `feat(dsp): polyphase decimation with Kaiser FIR`.
4. **Never edit the PRD or SDD directly.** Use `docs/proposals/` for changes.
5. **Never copy source from `Local/franken_doppler_finder`.** It is a functional
   reference only, git-ignored, with a different licence. Any reuse needs a
   proposal, a licence check and a `docs/third_party.md` entry first.
6. **Do not invent thresholds.** Use config values tagged `PROVISIONAL(G#)`
   until a gate fixes them; record the measurement that fixes them.
7. **Do not weaken a test, lint or CI step to get green.** Fix the cause.
8. **Tests are offline and seeded.** No network, no wall-clock dependence.
9. **A new dependency needs licence, purpose and replacement rationale** in
   `docs/third_party.md` before it lands.
10. **When blocked, record a question, take the SDD default and continue** with
    an independent task.

### Status markers used below

| Marker | Meaning |
| --- | --- |
| `[ ]` | Not started |
| `[~]` | In progress or partial |
| `[x]` | Done |
| `[!]` | Blocked; the reason is written beside it |

Update the marker and append the commit hash when a task closes. Do not
renumber tasks; append new ones instead.

---

## 3. Dependency order

```text
core-types
   └── geodesy
         └── acoustics
               └── dsp
                     └── events
                           └── search
                                 └── pipeline
                                       └── runtime
                                             └── api / worker

sim  ── uses acoustics + geodesy; never events/search
cli  ── uses pipeline + sim
store ── independent; consumed by runtime and api
```

`sim` staying independent of `events` and `search` is a hard rule: the test
oracle must not share a bug with the code it tests.

### Critical path to v0

```text
DSP → EVT → SRC → PIP → CLI → G1/G2
             ↑
GEO/ADSB ────┘
```

`adsb` can run in parallel with `dsp`/`events`. Do not start `search` until the
event model and fit covariance are stable.

### Critical path to v1

```text
STO → RUN → API → WEB → G3/G4/G5/G6
```

---

## 4. Milestones

| Milestone | Scope | Exit criteria |
| --- | --- | --- |
| M0 Foundations | `core-types`, `geodesy`, `acoustics` | Closed except DEM |
| M1 Simulator | `sim` | Closed; FR-SIM-01/03 done, residuals tracked |
| M2 DSP front end | `dsp` | Simulated audio → whitened z-map and ridge |
| M3 Event measurement | `events` | Detect and fit a simulated pass with covariance |
| M4 ADS-B, terrain and atmosphere | `geodesy`, `adsb`, `acoustics` | Offline fixture ingest, cache, terrain, geoid and measured atmosphere |
| M5 Search | `search` | Two-event point estimate, refusal, calibrated ellipse |
| M6 Pipeline and CLI | `pipeline`, `cli` | `sim`, `analyze`, `ingest`, `eval` work end to end |
| M7 v0 gates | validation | G1 and G2 pass; v0 requirements closed |
| M8 Store and runtime | `store`, `runtime` | Durable queue, leases, cancellation, retention |
| M9 API and worker | `api`, `worker` | v1 job lifecycle over HTTP |
| M10 Frontend and deploy | `web`, `deploy` | Public demo runs behind a proxy |
| M11 v1 gates | validation | G3, G4, G5, G6 pass |
| M12 v3 experiments | deferred | Only after the core is stable |

---

## 5. Phase plans

### M0 — Foundations

**Status:** `[x]` closed as a foundation; DEM remains open under M4.

Completed: unit newtypes, two clocks, fake clock, cancellation, IDs, manifest
types/hash, WGS84/ECEF/ENU, atmosphere with standard fallback, emission-time
Doppler model and retarded-time solver.

Residual work:

- **GEO-01** `[x]` Geoid model and orthometric-to-ellipsoidal conversion
  (FR-DA-02). Commit `74780dc`. See M4.
- **GEO-02** `[x]` DEM sampler and terrain lookup (FR-DA-02). Commit `3dfd22f`.

### M1 — Simulator

**Status:** `[x]` closed as a library; residuals are tracked below.

Completed: scenario model, emission-time tone synthesis, harmonics, seeded
white/pink noise, optional air absorption, WAV and truth export, degraded ADS-B
feed, event/position CRLB and a golden digest over `scenarios/example-single-pass.json`.

Residual work:

- **SIM-01** `[~]` Expand the regression suite beyond one scenario (FR-SIM-04).
  Add multi-pass, high/low SNR, multiple harmonics, absorption and feed-error
  cases. Store expected metrics, not audio, where possible. Fixture expansion can
  start now; the 1,000-scenario study is VAL-01 after M6.
- **SIM-02** `[~]` Replace the input-error sensitivity ranking with a
  position-error variance decomposition (FR-SIM-02). Blocked on M5 and M7.
- **SIM-03** `[ ]` Close Q-11 (SNR convention/noise colour) and Q-12
  (absorption coefficients) using measured archive/device data. Regenerate the
  golden digest and say why in the commit.
- **SIM-04** `[ ]` Add ground reflection only if it becomes necessary for the
  error budget; otherwise record it as deferred.
- **SIM-05** `[ ]` Add manoeuvring trajectories (turns and speed change within a
  pass) so the manoeuvre row of the G2 budget can be simulated (FR-SIM-01). If
  the scenario model cannot express it, extend the model and its tests first.

### M2 — DSP front end

**Status:** `[x]` closed: a simulated pass is decoded, decimated, transformed,
whitened and tracked end to end.

Read SDD section 5. The crate must stay synchronous and allocation-free inside
per-sample/per-bin loops.

- **DSP-01** `[x]` Media decoder seam and FFmpeg adapter (FR-IN-01, T-1).
  `MediaDecoder` is the seam; `FfmpegDecoder` runs the codec tool as a child
  process and never links its libraries. The child is asked for mono 32-bit float
  WAV on stdout, its stderr is drained so it cannot block, and the returned WAV
  is re-parsed and checked for sample format, channel count, rate, whole-sample
  length, finiteness, duration and decoded size. File size is checked before the
  process starts; the read is capped and the child is killed on timeout. The
  first line of `ffmpeg -version` is carried on `DecodedAudio` for the manifest.
  Tests: `media::tests::*`, including a real 0.5 s 48 kHz WAV decoded to 4 kHz
  and rejected-clip, oversize, missing, unsupported and absent-decoder cases.
  Upload and service limits remain API work under FR-IN-01. Commit `536ee05`.
- **DSP-02** `[x]` Band-limit, decimate and remove DC (FR-IN-01).
  Rational `L/M` polyphase resampling with a Kaiser-windowed FIR: 1.5 kHz
  passband, 1.9 kHz stopband edge and 80 dB attenuation by default, with a
  20 Hz high-pass at the analysis rate and DC removed after the rate change.
  Tests: `frontend::tests::*` in `crates/dsp`, covering 44.1 kHz and 48 kHz
  passband tones, stopband attenuation, aliasing probes, high-pass rejection of
  sub-audio tones, DC removal, determinism and rejected configurations.
  Commit `5292f9f`.
- **DSP-03** `[x]` Real STFT (FR-DM-01).
  Kaiser-windowed (`beta = 7.86`, about 80 dB sidelobes), 75% overlap, frame
  geometry from `StftConfig`, zero-padding to the next power of two at or above
  twice the frame length, magnitudes calibrated as an amplitude spectrum and
  parabolic log-magnitude interpolation for sub-bin frequency. One FFT plan and
  its scratch buffers are reused across frames; `StftConfig::survey(rate)` fixes
  the 1 s survey window.
  Tests: `stft::tests::*` and `window::tests::*` in `crates/dsp`, covering
  amplitude calibration against a known sine, recovery of a frequency 0.2 bin
  off centre, frame times, zero-padding, determinism and rejected geometry.
  Added `realfft` (3.5, with `rustfft`), recorded in `docs/third_party.md`.
  FR-DM-01 stays in progress until EVT-01 detects on the z-map. Commit
  `62b6e0d`.
- **DSP-04** `[x]` Noise whitening (FR-DM-01).
  Per-bin median and MAD over a centred window of frames (15 by default,
  `PROVISIONAL(G1)`), z values with roughly unit noise scale, and a MAD-to-sigma
  factor of 1.4826. A sigma floor of 1e-3 of the bin median keeps a silent bin
  from dividing by zero while still letting an onset register.
  `whiten_with_mask` lets the detector exclude a loud pass on the second pass.
  Median and MAD use in-place selection on reused buffers, so the frame loop
  allocates nothing.
  Tests: `whiten::tests::*`, covering pooled unit scale on white noise, a
  transient tone far above the floor, the masked second pass, silence, the
  sigma floor, determinism and rejected shapes.
- **DSP-05** `[x]` Ridge tracking ladder (FR-DM-01).
  `Corridor` supplies one inclusive bin range per frame. Level 1 takes the
  per-frame peak; level 2 runs Viterbi with a quadratic step penalty and a
  maximum step; level 3 runs a constant-velocity Kalman filter with an RTS
  smoother and reports per-frame variance in bins squared. `track_ridge` keeps
  the lowest level whose frames all clear `min_peak_z` and whose steps stay in
  budget. All thresholds are `PROVISIONAL(G1)`.
  Tests: `ridge::tests::*`, covering a clean ridge at level 1, a bright outlier
  rejected by Viterbi, a faint ridge smoothed with variances, corridor
  conversion and validation, and determinism.

- **DSP-06** `[x]` M2 exit test (FR-IN-01, FR-DM-01).
  `crates/dsp/tests/m2_exit.rs` writes a 16-bit 48 kHz pass with a known
  straight-line Doppler curve over seeded noise, decodes it with the FFmpeg
  adapter, resamples to 4 kHz, forms a survey spectrogram, whitens it and tracks
  the ridge through a corridor built from the same curve. Each frame must sit
  inside the band its one-second window spans, widened by two and a half bins.
  The measurement showed that greedy peaks land at the leading edge of the smear
  because whitening normalises each bin against its neighbouring frames, and that
  a pass sweeping far more slowly than the window's mainlobe is invisible to
  whitening at all; both are why the estimator fits the curve instead of picking
  peaks. One frame is trimmed at each end for the resampler's edge transient.
  Commit `1820e37`.

**Exit:** a simulated WAV is decoded, decimated, transformed, whitened and
tracked; a known ridge is recovered within a stated tolerance; results are
deterministic.

### M3 — Event measurement

**Status:** `[x]` done; EVT-01 to EVT-05 and the end-to-end exit test pass.

Read SDD section 5. `events` may depend on `dsp`, `acoustics` and `core-types`,
never on `search` or `pipeline`.

- **EVT-01** `[x]` Doppler detector (FR-DM-01). Commit `f1cc2ab`.
  Generalized likelihood-ratio template correlation over speed, closest
  distance, time and base frequency; require both approach and recede halves to
  pass; set the threshold by a Neyman–Pearson rule at a configured false-alarm
  rate per hour; estimate the null from time-reversed and frequency-shifted maps.
  `detect` searches the grid, scores each template as the mean z along its curve
  and reports the best hypothesis. A hypothesis is eligible only with at least
  `min_frames_per_half` frames in each half, so a curve cannot win by covering
  one side alone. Thresholds come from the same search run on a frame-reversed
  map and on maps rotated by one eighth and one quarter of the bin count; the
  false-alarm rate per hour sets the empirical quantile, which with a handful of
  null maps is the maximum and therefore conservative. The default grid is
  `PROVISIONAL(G1)`. `dsp` now exposes `ZMap::from_parts` so a fixture can hold
  exactly known contents.
  Tests: `detect::tests::*`, covering detection of an injected curve, rejection
  of noise, rejection of a one-sided ramp, determinism, settings validation, a
  map too short for two halves, and the grid product.
- **EVT-02** `[x]` Track-then-fit estimator (FR-DM-02, FR-DM-04). Commit
  `c07323d`.
  Joint nonlinear least squares over visible harmonics, weighted by per-frame
  variance, robust loss, LM/trust-region optimizer, multi-start from the
  detector. Covariance `(JᵀWJ)⁻¹` scaled by `max(1, χ²/dof)`; profile-likelihood
  intervals where `v` and `d` are strongly correlated. `HarmonicTrack::from_ridge`
  converts a `dsp` ridge on a `ZMap` into a variance-weighted track.
  Tests: `fit::tests::*`, covering recovery of a known pass with its covariance,
  tightening of the fundamental when harmonics are added, resistance to a single
  outlier, covariance inflation above the stated noise, profile intervals that
  bracket the estimate, recovery from a poor detector seed, determinism, input
  validation, harmonic zero and ridge conversion.
- **EVT-03** `[x]` Direct parametric estimator (FR-DM-05). Commit `59359cb`.
  Maximize whitened power sampled along the model curve, per harmonic and frame;
  Hessian covariance; use on faint events where ridges fail. The z values of
  the two bins around the model frequency are interpolated linearly and squared,
  giving the generalized likelihood ratio under exponential noise. A
  deterministic Nelder–Mead simplex handles the speed/distance valley that
  coordinate steps miss, and `(-H)⁻¹` gives the covariance at the optimum. The
  report reuses `FitReport` with `FitMethod::Direct`.
  Tests: `direct::tests::*`, covering recovery of the fitted curve from a faint
  map with no ridge, an objective that prefers the true pass over a detector
  seed, determinism, and rejected maps, harmonic lists and seeds.
- **EVT-04** `[x]` Quality score and gate (FR-DM-06). Commit `e755f8d`.
  Combine curve SNR, harmonic count, residual autocorrelation and covariance
  inflation. Drop low-scoring events and report why. The fitted curve is sampled
  with the direct estimator's interpolation and the mean whitened power gives
  the SNR, so noise alone sits near 0 dB. Four bounded components average into
  the score: SNR between the minimum and full-score thresholds, harmonic count
  against the target, inverse positive residual autocorrelation and inverse
  covariance inflation. Hard gates drop an event below any limit, and a
  combined-score cut catches weak events that pass each gate; the assessment
  keeps the ordered reasons and a one-line explanation for the report. The fit
  report now carries the lag-one residual autocorrelation, measured per harmonic
  and averaged. Thresholds are `PROVISIONAL(G1)`.
  Tests: `quality::tests::*`, covering retention of a strong event, rejection of
  noise with a reason, rejection for inflation and autocorrelation, determinism
  and settings validation.
- **EVT-05** `[x]` Physics regression (FR-DM-03). Commit `8b3d443`.
  Assert the emission-time model beats a delayed-copy control and that adding
  harmonics tightens the fit. Synthetic tracks are generated by the emission
  curve, fitted, then rescored with a control that evaluates radial speed at
  reception time. The emission residuals match the reported weighted `χ²`;
  the delayed copy misses by orders of magnitude more than the stated noise.
  The existing joint-harmonic test covers the tightening. The acoustics suite
  keeps its direct curve comparison green.
  Tests: `fit::tests::fr_dm_03_emission_time_model_beats_a_delayed_copy_control`
  and `acoustics::doppler` curve tests.
- **M3 exit** `[x]` Audio-to-score end-to-end test. Commit `4041628`; detector
  template fix `c813098`. `crates/events/tests/m3_exit.rs` synthesises a
  two-harmonic pass at the analysis rate on the emission-time curve, whitens the
  survey map, detects it, fits it directly and scores it. The detector must
  recover the grid truth, the fitted curve must stay inside one bin, the
  covariance diagonals must be usable and the event must be retained; a
  noise-only recording must be rejected.
  Tests: `m3_exit_a_simulated_pass_is_detected_fitted_and_scored` and
  `m3_exit_noise_alone_is_not_detected`.

**Exit:** a simulated pass is detected and fitted; recovered parameters agree
with truth; covariance is sane against the CRLB; low-SNR events are down-weighted
or dropped; no manual tracing. Covered by `crates/events/tests/m3_exit.rs`
(commit `4041628`).

### M4 — ADS-B, terrain and atmosphere

Read SDD sections 4, 6.1, 7 and 9.1–9.2. Network clients must never appear in
tests; use recorded fixtures.

- **GEO-01** `[x]` Geoid model (FR-DA-02). Commit `74780dc`.
  `GeoidGrid` holds a regular latitude/longitude grid of undulations in metres
  with name, version, source and licence for the manifest (NFR-04, NFR-06).
  Sampling is bilinear, edges are inclusive and anything outside returns
  `NoCoverage`. `GeoidModel` now converts orthometric to ellipsoidal height and
  back with `h = H + N` from SDD section 6.1, so every model shares the datum
  convention. The text loader reads the documented `doppler-geoid` v1 format
  without a new crate. A 13 by 13 EGM96 subset over 50–53 N, 2 W–1 E ships as
  test data, extracted from PROJ's public-domain `egm96_15.gtx`; the format and
  provenance are in `docs/geoid-grid-format.md`.
  Tests: `crates/geodesy/tests/geoid.rs`, covering a hand-computed bilinear
  cell, EGM96 node reference values, height round trips, coverage and invalid
  coordinates, metadata, malformed grids and text, and inclusive edges.
- **GEO-02** `[x]` DEM sampler (FR-DA-02). Commit `3dfd22f`.
  `DemGrid` holds a regular latitude/longitude grid of orthometric heights in
  metres with product, version, source, licence and vertical error for the
  manifest (NFR-04, NFR-06). Sampling is bilinear, edges are inclusive, a void
  or anything outside returns `NoCoverage`, and a cell with any void corner is
  never interpolated. The loader reads the documented `doppler-dem` v1 text
  format and decompressed SRTM `.hgt` tiles: big-endian `i16`, north-first rows
  flipped into the south-first convention, and `-32768` mapped to a void.
  `SrtmTile` parses names such as `N27E086`, and `observer_height_m` composes
  terrain, ear height and geoid undulation as `h = H_terrain + H_ear + N` from
  SDD section 6.1. A 33 by 33 SRTMGL1 crop around Everest ships as test data;
  the format and provenance are in `docs/dem-grid-format.md`.
  Tests: `crates/geodesy/tests/dem.rs`, covering a hand-computed bilinear cell,
  SRTM node reference values, inclusive edges, coverage and voids, metadata and
  vertical error, big-endian row flipping and void mapping, bad byte lengths,
  tile-name parsing, observer-height composition, and malformed grids and text.
- **ADSB-01** `[x]` Archive clients (FR-DA-01). Commit `b915978`.
  `OpenSkyClient` implements `TrackSource` over the state-vector endpoint and
  converts state vectors into time-ordered tracks. Every request goes through
  `HttpPolicy`: HTTPS only on `opensky-network.org`, the default port, no
  embedded credentials and no fragment; `ReqwestTransport` uses the blocking
  client with `rustls-tls-native-roots`, a timeout, refused redirects and a
  streaming size cap. Statuses 408, 425, 429, 500, 502, 503 and 504 are retried
  up to a configured bound; other failures return immediately. Samples without
  a position, geometric altitude or timestamp, surface reports, and points
  outside the query window are dropped. `sampling_stats` measures the median
  and nearest-rank 90th percentile of positive sample gaps and builds the
  `DataSource` manifest record. No test opens a socket; the client is exercised
  through a scripted transport and a documented-shape OpenSky fixture. The
  REST endpoint serves recent states only, so full-day history remains open
  for the Trino/dump ingest work. Reported position latency is recorded:
  `TrackSample::latency_s` carries the `time_position`/`last_contact` gap and
  `sampling_stats` surfaces median/p90 into `DataSource` (ADSB-05).
  Tests: `crates/adsb/tests/da_01.rs`, covering parsing, window filtering,
  malformed payloads, empty states, allow-list rejection, the size cap, bounded
  and non-retryable responses, request construction, query validation and
  sampling statistics. Source notes are in `docs/adsb-sources.md`.
- **ADSB-02** `[x]` Parquet track store and cache (FR-DA-04).
  `TrackStore` writes the SDD 9.2 schema partitioned by source, UTC date and
  1° tile at `source=<kind>/date=YYYY-MM-DD/lat=<floor>/lon=<floor>/part-<sha256>.parquet`,
  keeping both `alt_geom_m` and `alt_baro_m`, callsign, type code, vertical
  rate and quality flags. File names are content hashes, so identical input
  gives identical paths and bytes. Replay prunes directories by date and tile,
  filters rows by the exact inclusive window and region, preserves the
  original timestamps without resampling, and regroups split tracks by source
  and ICAO24. `ingest_runs` recomputes median and p90 sampling gaps per source
  and day from the files on disk and records each file's SHA-256. Tests:
  `crates/adsb/tests/da_04.rs`, covering both altitude datums, partition paths,
  region/window pruning, exact timestamps, ingest statistics, deterministic
  writes, invalid input, empty input and flag round-trips. Store notes are in
  `docs/track-store.md`.
- **ADSB-03** `[x]` Hermite interpolation, legs and closest approach (FR-DA-01).
  `TrackInterpolator` replaces the linear seed with cubic Hermite
  interpolation in a track-local ENU frame; knot velocities are
  centred finite differences of stored positions, and
  `position_ecef(unix_time_s)` is the seam `acoustics::solve_emission_time`
  plugs into. `Leg` derives the SDD 9.2 legs row — deterministic
  `leg_id`, time span, bounding box, point count, measured median and maximum
  sampling gaps, and an engine class from a starter ICAO type-code table
  (unlisted codes stay `Unknown`, Q-15). `closest_approach` minimizes 3-D
  slant range over the interpolant with a per-interval golden-section search
  and applies `t_r = t_e + range / c` exactly, returning emission and heard
  times, slant range and horizontal speed. Tests:
  `crates/adsb/tests/track_model.rs` plus interpolate unit tests, covering
  knot exactness, a curved-leg accuracy check against the chord, leg fields
  and engine classes, closest-approach geometry and input rejection. Model
  notes are in `docs/track-model.md`.
- **ADSB-04** `[x]` Source/region support flags (T-4).
  `assess_coverage` reduces an ingest result to `Supported`,
  `Insufficient` or `Unsupported` from measured sampling gaps, so an empty
  track set is never silently treated as complete. The median-gap bound
  (`MAX_SUPPORTED_MEDIAN_GAP_S` = 10 s) is `PROVISIONAL(T-4)` pending the
  measured per-source statistics; unmeasured regions stay `Unsupported`,
  matching the T-4 default for Addis. Tests: `crates/adsb/tests/coverage.rs`,
  covering empty, dense, sparse, single-sample and boundary ingests.
- **ACO-01** `[x]` Measured atmosphere adapter (FR-DA-03).
  `acoustics::weather` ingests the normalized weather payload, averages
  conditions and the wind vector over a window, projects the wind onto the
  source-observer path with the Q-10 sign fixed, and falls back to the
  flagged standard atmosphere when no sample exists. The live fetch adapter
  is separate work; tests replay a recorded fixture only. Tests:
  `crates/acoustics/tests/weather.rs`.
- **ADSB-05** `[x]` Full-day history ingest (FR-DA-01).
  Position-latency recording landed first: `TrackSample::latency_s`, the
  Parquet `latency_s` column (schema v2) and
  `DataSource::latency_median_s`/`latency_p90_s`. `adsb::dump` then ported
  the adsb.lol `globe_history` ingest from the reference implementation:
  tar walking with end-marker validation, gzipped `trace_full_*` members,
  lenient readsb row visitors, window/region filtering with interpolation
  anchors, and `AdsbLolArchive` as a `TrackSource` over local archives.
  Release discovery/download stays fetch-layer work; readsb carries no
  staleness gap so dump samples report `latency_s: None` (Q-16).
- **ACO-02** `[x]` Live weather fetch adapter (FR-DA-03).
  `adsb::weather` fetches Open-Meteo hourly surface conditions through the
  same `HttpPolicy`/`HttpTransport` guard as the ADS-B clients and emits
  the normalized payload `acoustics::weather::parse_weather` ingests —
  the payload string is the seam, so `acoustics` stays I/O-free. Both the
  ERA5 archive and historical-forecast endpoints are selectable; missing
  hourly fields are skipped rather than fabricated. Tests replay a
  recorded response: `crates/adsb/tests/aco_02.rs`. Source notes:
  `docs/weather-source.md`.

**Exit:** a fixture archive ingests, caches and replays offline; the manifest
records source, version, hash and sampling statistics; terrain and geoid affect
the predicted slant range; the atmosphere is measured or explicitly flagged as a
fallback; licences are recorded in `docs/third_party.md`.

### M5 — Search and localization

Read SDD section 6. This is the first crate allowed to combine events, tracks,
terrain and time.

- **SRC-01** `[ ]` Prediction (FR-SL-01).
  For hypothesis `(observer x, flight j)`, find the closest approach in 3-D
  slant distance with the light-time solver; observer height = terrain + ear
  height; aircraft height uses geometric altitude via the geoid.
- **SRC-02** `[ ]` Event likelihood (FR-SL-01).
  Gaussian residual on `(v, d)` with `Σ_vd + Σ_adsb + Σ_model`; `f0` is a
  nuisance parameter; compute the log-likelihood.
- **SRC-03** `[ ]` Clock term (FR-SL-01).
  Each event implies a recording start `s_i`; events must agree within
  `σ_s,i`; a known time adds one Gaussian term. This replaces hard offset
  filtering.
- **SRC-04** `[ ]` Silence term (FR-SL-01).
  Build `P_detect(distance, aircraft type)` from the simulator detection ROC;
  score unexplained audible passes with a Poisson-binomial term; exclude jets by
  engine class and treat unknown classes probabilistically.
- **SRC-05** `[ ]` Coarse hypotheses and gate (FR-SL-01).
  Grid per event and flight; keep residuals inside a 4σ gate.
- **SRC-06** `[ ]` Pair join (FR-SL-01).
  Spatial hash of `(cell, clock bin)` so only cells shared by two events survive.
- **SRC-07** `[ ]` Posterior scoring (FR-SL-01).
  Full log-posterior over survivors; top-K clusters; log-sum-exp over
  assignments; land prior uniform over land in the region.
- **SRC-08** `[ ]` Continuous refinement (FR-SL-03).
  LM on the negative log posterior over `(lat, lon, s)` with land-mask and bounds
  constraints; covariance from the inverse Hessian; report separate clusters.
- **SRC-09** `[ ]` Calibration factor (FR-SL-04, NFR-01).
  Fit `κ` on simulation and keep validation clips held out; store `κ` and model
  version with every result.
- **SRC-10** `[ ]` Verdicts and refusal (FR-SL-02, FR-OUT-01).
  Implement the SDD 6.5 table: mass thresholds, at least two independent events
  or one event plus known time/silence, 95% semi-major axis ≤ 1 km, no truncation,
  no veto. One event with no known time is always `NotLocatable`. Mirror pairs are
  at most `Ambiguous`.
- **SRC-11** `[ ]` Ranked alternatives (FR-SL-05).
  Report ranked clusters with posterior mass and the margin over the best rival.
- **SRC-12** `[ ]` Truncation handling (FR-SL-01).
  Every stage has a work budget; a truncated stage sets a flag and caps the
  verdict at `Ambiguous`.

**Exit:** two simulated events with a known time gap recover the observer within
the predicted ellipse; a single event without a known time is refused; an exact
mirror pair is `Ambiguous`; 95% coverage is measured and calibrated.

### M6 — Pipeline and local CLI

Read SDD sections 9.4, 10.1 and 10.4. This milestone produces the v0 artifacts
needed by G1/G2.

- **PIP-01** `[ ]` Stage orchestration (FR-OUT-04).
  Run `ingest → decode → detect → fit → match → localize → render` with a
  `JobCtx`, cancellation checks at boundaries and inside long loops, progress
  callbacks and wall-time budgets.
- **PIP-02** `[ ]` Run manifest (NFR-04).
  Build and write the JSON from SDD 9.4: manifest version, git commit, crate
  versions, parameter hash, seed, data sources with version/hash/sampling,
  atmosphere source or fallback flag, model/`κ` versions, thread count, stage
  timings, truncation flags. Hash it with SHA-256.
- **PIP-03** `[ ]` Result payload and manifest export (FR-OUT-04).
  Export verdict, ranking, evidence and manifest JSON. Research mode may carry
  full geometry; the serialized shape is fixed here so the public type cannot
  later leak exact coordinates.
- **PIP-04** `[ ]` Public coarsening (FR-OUT-03, v1).
  One function builds public geometry at the configured cell size; no other code
  constructs a public location. The public result type has no exact-coordinate
  field.
- **PIP-05** `[ ]` Report figures (FR-OUT-02, v1).
  Spectrogram with fitted curves, matched flights, uncertainty and per-channel
  evidence. Use `plotters`/SVG; keep figures out of the compute hot path.
- **CLI-01** `[ ]` `sim` command (FR-SIM-01, FR-SIM-03, FR-SIM-04).
  Load a scenario JSON and seed; write WAV, truth JSON, degraded feed and CRLB
  report. Expose noise/absorption settings. This is the first end-to-end use of
  the existing simulator.
- **CLI-02** `[ ]` `analyze` command (FR-IN-02, FR-IN-03, FR-OUT-04).
  Media path, local date/UTC window, coarse region, optional known time; run the
  pipeline; write result JSON and manifest; print the verdict and evidence.
- **CLI-03** `[ ]` `ingest` command (FR-DA-01, FR-DA-04).
  Source, date window and region; fill the cache; record source statistics.
- **CLI-04** `[ ]` `eval` command (G1, G2, G3).
  Re-run a scenario suite or manifest set; emit coverage, error and confusion
  tables as machine-readable JSON plus a human summary.
- **CLI-05** `[ ]` Progress, cancellation and offline mode (NFR-02, NFR-04).
  Ctrl-C cancels cleanly; progress goes to stderr; `--offline` uses caches only.

**Exit:** `cli sim` produces all artifacts; `cli analyze` runs a simulated WAV
through the full pipeline; the same manifest reproduces the same result; `cli
eval` computes a coverage table.

### M7 — v0 validation and gates

This is the release gate for v0. Do not start v1 service work before it passes.

- **VAL-01** `[ ]` Scenario suite (G1).
  1,000 scenarios spanning SNR, distance, speed, harmonics, noise colour,
  absorption, sampling/latency, quantization, altitude datum, terrain, wind,
  temperature, RPM drift and manoeuvre. Store expected metrics beside scenarios;
  tolerance failures fail CI.
- **VAL-02** `[ ]` G1 coverage (G1).
  Run the suite; count 95% regions containing truth; target at least 90%. Publish
  the coverage table and script output.
- **VAL-03** `[ ]` G2 error budget (G2, FR-SIM-02).
  Run baseline, one-at-a-time (three magnitudes) and combined profiles through
  the real estimators; decompose position variance by source; publish the ranked
  table. Replace the input-error ranking in `sim::inject`.
- **VAL-04** `[ ]` Fix provisional values (G1, G2).
  Set detector thresholds, quality cuts, verdict thresholds, `κ`, absorption
  coefficients and ADS-B error ranges from measurements. Remove `PROVISIONAL`
  tags only with the measurement recorded.
- **VAL-05** `[ ]` Close questions (Q-11, Q-12, Q-13).
  Record the decisions in `docs/questions.md`; regenerate the golden digest when
  the synthesis convention changes.
- **VAL-06** `[ ]` Update the scorecard.
  Mark every v0 row `done` or explicitly deferred with a reason. Update
  `docs/validation-plan.md`, `docs/traceability.csv` and the README status line.

**Exit:** G1 ≥ 90% coverage; G2 ranked table complete and reviewed; v0 CLI and
manifests reproduce; no v0 requirement silently open.

### M8 — Store and runtime (v1 foundation)

Read SDD sections 9.3, 9.5, 9.6, 10.1 and 11.6. Start only after M7.

- **STO-01** `[ ]` sqlx setup and migrations (FR-AC-04).
  SQLite now, portable SQL for Postgres later; tables from SDD 9.3; committed
  offline query data; migrations never edited after application.
- **STO-02** `[ ]` Repositories (FR-AC-04, FR-OP-02).
  Jobs, job events, sessions, limits, results, manifests, rate limits, ingest
  runs, sim runs. Parameterised queries only; state transitions validated.
- **STO-03** `[ ]` Retention and limits (FR-AC-04).
  Implement the retention sweeper and the SDD 11.3 limits; no raw IP storage.
- **RUN-01** `[ ]` Claim loop and lease heartbeat (FR-OP-01).
  Atomic claim from the `jobs` table, lease expiry, requeue, heartbeat.
- **RUN-02** `[ ]` Job execution and outcome mapping (FR-OP-01).
  `run_job` with deadline, cancel bridge, panic containment and the SDD 11.6
  outcome table; blocking work via `spawn_blocking`.
- **RUN-03** `[ ]` Shutdown and drain (FR-OP-01).
  SIGTERM stops claiming, drains or cancels running jobs, releases leases,
  requeues unfinished jobs once.

**Exit:** fake-clock tests prove claim-once, lease expiry, cancellation, panic
containment, media deletion and terminal-row consistency; no detached task.

### M9 — API and worker (v1)

Read SDD sections 10.2, 10.3, 11.1–11.4 and 12.

- **API-01** `[ ]` Config and error envelope (FR-OP-02).
  Load `config/default.toml` with validated overrides; one stable error code
  mapping; `not_locatable` is a successful result, not an error.
- **API-02** `[ ]` Sessions and attestation (FR-AC-01, FR-AC-03, FR-AC-05).
  Anonymous cookie session, attestation challenge, bot challenge.
- **API-03** `[ ]` Job submission and admission limits (FR-AC-02, NFR-08).
  Multipart or URL submission; enforce size, duration, rate, attestation and
  queue depth before reading the body; return `202` with job id and result token.
- **API-04** `[ ]` Job status, results and SSE (FR-OP-01, FR-OUT-01).
  Token-as-capability, stage/progress, result payload, SSE replay with
  `Last-Event-ID`, figure links, idempotent `DELETE`, `no-store` caching.
- **API-05** `[ ]` Rate limits and compute budget (FR-AC-02).
  Keyed daily-salted IP hash plus session key; both limits must pass; global
  daily budget and concurrency cap.
- **API-06** `[ ]` Media safety and SSRF (FR-AC-04, NFR-08).
  tmpfs scratch with deletion guard and sweeper backstop; decoder subprocess with
  CPU/memory/time limits; URL allow-list, private-range blocking, redirect rules.
- **API-07** `[ ]` Health, version and metrics (FR-OP-03).
  Liveness, build info and Prometheus metrics with no media, raw IPs, tokens or
  exact public coordinates.
- **WRK-01** `[ ]` Worker binary (FR-OP-01).
  Wire the claim loop, job scopes, retention sweeper and graceful shutdown.

**Exit:** integration tests with a fake clock and fixture ADS-B cover session,
submit, progress, result, delete, limit and retention paths; 10 concurrent jobs
complete with no leaked media.

### M10 — Frontend and deployment (v1)

Read `docs/ui_ux.md`, SDD 11.4 and 12. Resolve Q-6 before scaffolding.

- **WEB-01** `[ ]` Approve the frontend specification (Q-6).
  Page states, wording, privacy copy and the result layout.
- **WEB-02** `[ ]` Scaffold Nuxt/bun (`web/`).
  Upload, region/date form, session, progress, result page, evidence figures.
- **WEB-03** `[ ]` Public-mode presentation (FR-OUT-03).
  Region cell and confidence only; never imply metre-level public accuracy.
- **DEP-01** `[ ]` Deployment (SDD 11.4).
  Dockerfile, reverse proxy/TLS, SQLite volume, backups, artifact storage,
  metrics scrape, health checks.
- **OPS-01** `[ ]` Operations.
  Runbook, abuse response, deletion route, retention checks, backup restore test.

**Exit:** a public upload runs through the API and worker and renders a coarsened
result; no exact coordinates or media appear in logs or public results.

### M11 — v1 gates

- **CAL-01** `[ ]` Real-clip dataset (G3, G4).
  At least 10 held-out clips with known location/time and recorded consent and
  licence; US first, Addis second. Keep them out of tuning.
- **CAL-02** `[ ]` Calibration (G3, FR-SL-04).
  Fit/validate `κ`; produce a reliability plot and per-clip error table; coverage
  at least 90%.
- **CAL-03** `[ ]` Refusal gate (G4).
  Single-event clips without a known time are refused 100%; confident wrong
  answers at most 5% of answered clips.
- **CAL-04** `[ ]` Demo robustness and cost (G5, NFR-03).
  10 concurrent jobs on one node; no failures, no leaked media; record the
  per-job compute and memory profile that sets the service cap.
- **CAL-05** `[ ]` Reproducibility (G6, NFR-04).
  Regenerate the results chapters from manifests; 100% hash match.
- **CAL-06** `[ ]` Update the scorecard and release notes.
  All v1 rows done or explicitly deferred; README status and docs updated.

### M12 — v3 experiments (deferred)

Do not start these until the core is stable and G1–G6 are closed.

- **EXP-01** `[!]` Grid-hum channel (FR-EX-01), deferred to v3.
- **EXP-02** `[!]` Ground-reflection height channel (FR-EX-02), deferred to v3.
- **EXP-03** `[ ]` Bell beacons, Ethiopia case study; each needs its own gate.

---

## 6. Current next tasks

The immediate unblocked queue, in order:

1. **X-02** local `cargo deny`/`cargo audit` parity: the configuration exists,
   the tools are not installed on every machine (`cargo audit` is missing
   locally; `cargo deny` runs but flags a pre-existing `tiny-keccak` CC0-1.0
   licence rejection unrelated to recent work).

Do not begin **SRC-01** until `EventFit` covariance and the detector output are
stable. Do not begin M8 before G1/G2 pass.

---

## 7. Known traps

| Area | Trap | Rule |
| --- | --- | --- |
| Acoustics | Using arrival-time velocity and delaying the curve | Always solve emission time; the curve is not a shifted copy (SDD 4) |
| Acoustics | Wind sign | Positive means source-to-observer and shortens propagation (Q-10) |
| Acoustics | Speed of sound as a literal | Take it from the atmosphere model |
| Sim | Golden digest changes silently | Regenerate only for an intended change and explain why in the commit |
| Sim | Shared bug with the estimator | `sim` must never depend on `events` or `search` |
| DSP | Allocation inside sample/bin loops | Reuse buffers and FFT plans |
| DSP | Detecting before whitening | Whiten first; second-pass noise after masking |
| Events | Ignoring `v`/`d` correlation | Use profile-likelihood intervals where needed |
| Search | Reporting a point from one event | One event is a curve; refuse unless known time/silence resolves it |
| Search | Mirror ambiguity | A single straight pass has two observer sides; never `Locatable` without breaking the tie |
| Search | Truncated search | Cap the verdict at `Ambiguous` and set the flag |
| Search | Calibrating on validation clips | `κ` is fitted on simulation; real clips stay held out |
| Pipeline | Manifest omissions | Any input that changes the result belongs in the manifest |
| Pipeline | Coarsening in several places | One function; public result type cannot hold exact coordinates |
| Store/runtime | Raw IP storage | Hash with the daily keyed salt inside the middleware only |
| Store/runtime | Non-atomic claim | One atomic update; lease expiry requeues |
| API | Reading the body before limits | Size/duration/rate/attestation checks come first |
| API | Logging media/tokens/coordinates | Structured fields only; public mode has no exact coordinates |

---

## 8. Cross-cutting backlog

These can land with any milestone, but must not distract from the critical path:

- **X-01** `[ ]` Traceability CI checker (NFR-07): fail CI when a requirement row
  has no test or an invalid status.
- **X-02** `[x]` `cargo-deny`/`cargo-audit` local instructions and CI parity.
  The norms now name the one-time `cargo install cargo-deny cargo-audit
  --locked`, state that both tools read the repository's `deny.toml` (the same
  file `cargo-deny-action` and `rustsec/audit-check` use in CI), and require a
  missing gate to be reported rather than skipped silently. `cargo deny check`
  was run on this tree: advisories, bans, licences and sources all pass, with
  warnings only for internal workspace path dependencies counted as wildcards
  and for the `ISC` and `Unicode-DFS-2016` allowances no crate currently uses.
  `cargo audit` could not be installed here; `cargo deny`'s advisory check reads
  the same RustSec database, so the vulnerability gate did run.
- **X-03** `[ ]` Criterion benchmarks for `dsp` and `search` before optimization.
- **X-04** `[ ]` Property tests for transform round trips, whitening and
  likelihood maximality.
- **X-05** `[ ]` Keep `docs/third_party.md` current as dependencies land
  (NFR-06).
- **X-06** `[ ]` Keep `docs/questions.md` current; never leave an ambiguity only
  in a code comment.
- **X-07** `[ ]` README status line and crate table updated at each milestone.

---

## 9. Open questions that affect sequencing

| Question | Affects | Default until resolved |
| --- | --- | --- |
| Q-1 public clip cap vs two-flyover assumption | M9, M7 limits | 90 s provisional; count real clips |
| Q-2 mirror ambiguity | SRC-10 | both positions stay; at most `Ambiguous` |
| Q-3 approved crate list | all | add per task with `third_party.md` entry |
| Q-4 TLS roots for `reqwest` | ADSB-01 | `rustls-tls-native-roots`; resolved in ADSB-01 |
| Q-5 public wording | WEB-03 | say “region”, never exact location |
| Q-6 frontend spec | WEB-01 | no frontend before approval |
| Q-10 wind sign/averaging | ACO-01 | source-to-observer positive |
| Q-11 noise/SNR convention | SIM-03, DSP-04 | peak-to-RMS, pink via three low-passes |
| Q-12 absorption coefficients | SIM-03 | none by default |
| Q-13 error-budget ranking | VAL-03 | input-error ranking is not the G2 result |
| T-1 decoder approach | DSP-01 | restricted FFmpeg subprocess |
| T-2 atmosphere source | ACO-01 | standard atmosphere, flagged |
| T-3 terrain datum | GEO-01/02 | public DEM/geoid with measured error |
| T-4 Ethiopia coverage | ADSB-04 | region unsupported until measured |
| T-5 track store | ADSB-02 | Parquet + Arrow |
| T-6 `κ` calibration set | SRC-09, CAL-02 | simulator plus first held-out clips |
| T-7 grid-hum reference | M12 | none; needs author recording |

---

## 10. Roadmap maintenance

- Update the status snapshot at the top when a milestone closes.
- Mark tasks in place; do not renumber.
- Append the commit hash and test name to a closed task.
- If a task splits, add `DSP-03a`/`DSP-03b` rather than reusing an ID.
- If a task is blocked, mark `[!]` and name the blocking task or question.
- Keep this file within the repository rules: no secrets, no generated data, no
  claims that were not measured.
