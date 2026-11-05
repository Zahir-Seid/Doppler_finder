# Doppler Locatability Audit

Locate an audio or video recording from the Doppler curves of propeller aircraft, matched against archived ADS-B flight tracks.

This repository is the v0 research core: a Rust workspace, a simulator-first test strategy, and a CLI/library pipeline. The estimator is not implemented yet; the scaffold fixes the crate boundaries, lints and documentation so each requirement can be built and tested in order.

## Documentation

- [Roadmap](roadmap.md) — execution order, current state and per-task acceptance criteria.
- [PRD](docs/PRD.md) — what and why, requirements, gates, risks.
- [SDD](docs/SDD.md) — architecture, methods, data design, API, operations.
- [Rust Working Norms](docs/rust-working-norms.md) — toolchain, lints, tests, definition of done.
- [Traceability](docs/traceability.csv) — requirement to section to test mapping.
- [Validation plan](docs/validation-plan.md) — datasets and acceptance tests.
- [Questions](docs/questions.md) — open decisions.
- [Third-party inventory](docs/third_party.md) — code and data licences.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `core-types` | Units, two clocks, ids, cancellation, run-manifest types |
| `geodesy` | WGS84, ECEF/ENU, DEM and geoid traits |
| `acoustics` | Atmosphere, light-time and emission-time Doppler model |
| `dsp` | Resampling, windows, STFT, whitening, ridge tracking |
| `events` | Detection, per-event fit, covariance, quality |
| `adsb` | Open-archive ingest, Parquet track store, interpolation |
| `search` | Hypotheses, likelihood fusion, refinement, verdicts |
| `sim` | Scenario model, synthesis, error injection, CRLB |
| `pipeline` | Stage orchestration, progress, manifests, reports |
| `store` | Relational repositories, limits, retention |
| `runtime` | Tokio task machinery shared by the binaries |
| `api` | Axum routes, SSE, rate limiting |
| `worker` | Queue worker binary |
| `cli` | Local analysis, simulation, ingest and evaluation |

Dependencies are added per implementation task from the approved list in the norms; the scaffold builds with the minimum needed for the binaries.

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p api
cargo run -p cli -- --help
```

Install `cargo-deny` and `cargo-audit` to run the full CI set locally.

## Licence

AGPL-3.0-only. See [LICENSE](LICENSE).
