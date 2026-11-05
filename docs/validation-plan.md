# Validation plan

This plan turns the PRD gates and the SDD test levels into concrete artifacts.
It is a living document: datasets are added with their licence and version, and
thresholds are fixed only after the v0 measurement they depend on.

## Test levels

| Level | What it proves | Where | Status |
| --- | --- | --- | --- |
| Unit | Units, coordinate round trips, closed-form Doppler versus a numeric retarded-time solver, window and filter properties | Each crate `tests/` and `#[cfg(test)]` | Not started |
| Property | Whitening leaves noise white; likelihood is maximal at the truth; transforms invert | `proptest` suites in `dsp`, `geodesy`, `search` | Not started |
| Estimator | Bias, variance against the CRLB, coverage of the 95% region | `sim` plus `pipeline` | Not started |
| Scenario regression | A fixed scenario suite with stored expected metrics; tolerance failures fail CI | `scenarios/` plus CI | Not started |
| Integration | API with fake clock and fixture ADS-B; rate limits, retention, SSE replay | `api` tests | Not started |
| Real data | Held-out clips with known location and time | `cli eval` | Not started |
| Load | Admission, queue depth and wall-time limits | staging | Not started |

## Datasets

| Dataset | Purpose | Size target | Licence / consent | Status |
| --- | --- | --- | --- | --- |
| Simulated scenarios | G1, G2, estimator coverage | 1,000 scenarios for G1 | Generated in-repo | Not started |
| Golden DSP fixtures | Front-end regression | Under 1 MB per fixture | Generated in-repo | Not started |
| US real clips | G3, G4 | At least 10 held-out clips | Contributor consent | Not started |
| Addis Ababa clips | G3 secondary set | As available | Contributor consent | Not started |
| OpenSky / adsb.lol snapshots | Matching and ingest | One state, one day for NFR-02 | Verify terms before caching | Not started |
| DEM and geoid tiles | Altitude and terrain | Region-specific | Public domain or open | Not started |

## Gates

Gate definitions and thresholds are owned by the PRD section 8. This table says
only how each gate is measured.

| Gate | Measurement | Acceptance | Artifact |
| --- | --- | --- | --- |
| G1 Simulator recovery | Run 1,000 scenarios; count 95% regions containing truth | At least 90% | Coverage table and script output |
| G2 Error budget | Injected-error study with variance decomposition | Complete and reviewed | Ranked table and report |
| G3 Real-clip calibration | Held-out clips with known location; reliability plot | At least 10 clips, at least 90% coverage | Per-clip table and `kappa` |
| G4 Refusal | Single-event clips without known time and ambiguous scenarios | 100% refusal; confident wrong answers at most 5% | Confusion table |
| G5 Demo robustness | 10 concurrent jobs on one node | No failures, no leaked media | Load-test report |
| G6 Thesis reproducibility | Re-run results chapters from manifests | 100% hash match | Hash comparison report |

## Acceptance tests by requirement

The full mapping is in [traceability.csv](traceability.csv). A requirement is
`done` only when its test exists, passes in CI, and the status row is updated in
the same pull request.

Minimum test intent:

- **FR-IN-01:** accepted and rejected media, oversize and overlong inputs, and
  a decoder timeout.
- **FR-IN-02/03:** region and time filters; a known start time narrows the
  posterior without excluding the truth.
- **FR-DM-01 to 06:** a known simulated pass is detected, fitted and scored;
  the emission-time model beats a delayed-copy control; harmonics improve the
  fit; low-SNR events are down-weighted.
- **FR-DA-01 to 04:** a fixture archive ingests, caches, replays offline and
  records its source statistics.
- **FR-SL-01 to 05:** soft likelihood fusion, refusal, covariance coverage,
  calibration and ranked alternatives.
- **FR-OUT-01 to 04:** verdict reasons, figure contents, public coarsening and
  manifest export.
- **FR-SIM-01 to 04:** synthesis, error injection, CRLB and golden scenarios.
- **FR-AC-01 to 05:** anonymous session, limits, challenge, deletion and
  attestation.
- **FR-OP-01 to 03:** queue progress, cancel, privacy-safe logs and health.

## Known risks to resolve in v0

1. **A2 versus the 90 s demo cap.** Count how many real clips contain at least
   two qualifying flyovers. If the count is low, keep the demo at "region"
   only and reserve point fixes for longer research clips (Q-1).
2. **One-event mirror ambiguity.** Add an exact mirror scenario; the verdict
   must not be `Locatable` unless the tie is broken (Q-2).
3. **Microphone low-frequency roll-off.** Many phones attenuate propeller
   fundamentals. Validate the harmonic comb on real devices before fixing the
   20 Hz high-pass.
4. **ADS-B coverage in Ethiopia.** Measure per-source sampling before marking
   the region supported (T-4).
5. **Calibration set for `kappa`.** Simulator-only calibration is not enough
   for G3; the first held-out real clips must be kept out of tuning.

## CI checks

The workflow in `.github/workflows/ci.yml` runs formatting, clippy with
warnings denied, tests, `cargo deny check` and `cargo audit`. The traceability
checker is not implemented yet; until it is, the reviewer enforces that every
new requirement row has a test ID and a status.
