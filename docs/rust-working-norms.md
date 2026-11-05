# Rust Working Norms

Nov 4, 2026

These rules bind every line of Rust written for the Doppler locator, whoever or whatever writes it. The PRD says what to build, the SDD says how it is structured, and this document says how the code is written and when it counts as finished. If two documents disagree, the PRD wins over the SDD, and the SDD wins over this document.

## 1. How to use this document

- **MUST** and **MUST NOT** are enforced by CI or review. **SHOULD** can be broken with a one-line reason in the code or the pull request.
- Read the SDD sections named in the task before writing code. Cite the requirement id (for example FR-DM-02) in the pull request.
- If the SDD is silent or ambiguous, take its stated default, record the question in `docs/questions.md`, and keep going. Do not invent behaviour and do not stop the whole task.
- Do not change the PRD or SDD. Propose a change as a file in `docs/proposals/`.
- Do not paste code from other repositories. Anything copied needs its licence and source recorded in `docs/third_party.md` first (PRD decision D-2).

## 2. Toolchain and workspace

- **Toolchain.** `rust-toolchain.toml` pins stable Rust 1.99.1 with `rustfmt` and `clippy`. Edition 2024. No nightly features.
- **Workspace.** One Cargo workspace with the crates and dependency direction of SDD section 3. Shared versions, lints and package metadata live in `[workspace.package]`, `[workspace.dependencies]` and `[workspace.lints]`; member crates inherit them.
- **Direction.** Library crates never depend on `api`, `worker` or `cli`. No cycles. A crate that needs something from a higher layer takes it through a trait defined lower down.
- **No unsafe.** `unsafe_code = "forbid"` for the whole workspace, with no exceptions. Enabling it would require changing this rule through a proposal.
- **Lockfile.** `Cargo.lock` is committed. CI builds with `--locked`.
- **One command set.** These run on every pull request and must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
cargo audit
```

## 3. Style, lints and documentation

- **Format.** Default `rustfmt`, no custom config. Never hand-format around it.
- **Lints.** Workspace `clippy::all` and `clippy::pedantic` at warn, with a short allow-list in `Cargo.toml`, each entry with a reason. `#[allow(...)]` in code needs a comment saying why.
- **No panics in library code.** `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and unchecked indexing (`v[i]`) are denied in library crates (`clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing`). Tests, benches and `main` startup may use `expect` with a message.
- **Naming.** Standard Rust conventions. Physical quantities carry their unit in the type or the name (`freq_hz`, `dist_m`, `t_audio_s`, `t_utc_ms`).
- **Size.** Functions over about 60 lines and modules over about 500 lines get split unless one clear algorithm needs the length.
- **Docs.** `missing_docs` is a warning on every public item. A function that implements a formula links the SDD section and states its units, valid range and failure cases. Comments say why, not what.
- **No dead code.** No commented-out code, no unused feature flags, no `TODO` without a requirement id or issue number.

The lint policy is configured, not only stated. `clippy -D warnings` does not turn on the restriction lints above by itself, and `missing_docs` is a compiler lint, not a Clippy one. The workspace root carries:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
indexing_slicing = "deny"
disallowed_methods = "deny"
```

Every member crate sets `[lints] workspace = true`. A `clippy.toml` at the root relaxes the rules for tests and bans detached tasks:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-indexing-slicing-in-tests = true
disallowed-methods = [
  { path = "tokio::spawn", reason = "use a Scope or JoinSet (SDD 11.6)" },
]
```

Binary startup code that must fail fast uses a local `#[allow(clippy::expect_used)]` with a reason comment.

`disallowed_methods` is a restriction lint that is off by default, so the line above is what makes the `tokio::spawn` ban fire. The test relaxations in `clippy.toml` do not cover `benches/`; each bench file starts with an `#![allow(...)]` for the same lints, with a reason comment.

## 4. Errors and logging

- **Libraries** return `Result<T, E>` with one `thiserror` enum per crate. Variants carry data, not formatted strings. Public error enums are `#[non_exhaustive]`.
- **Binaries** (`api`, `worker`, `cli`) may use `anyhow` at the top level only, to add context and print. Nothing below them returns `anyhow::Error`.
- **API codes.** Every error that reaches a client maps to one stable code from SDD section 10.3 in a single function in `api`. A new failure mode adds a code there, never an ad-hoc string.
- **Not locatable is a result, not an error.** It is a successful job with a verdict (SDD 10.3).
- **No swallowed errors.** `let _ = fallible()` and `.ok()` on a `Result` need a comment. Use `?` with `.context()` or a typed `map_err`.
- **Logging** uses `tracing` with structured fields and one span per job carrying `job_id` and `stage`. Levels: `error` needs a human, `warn` is degraded but served, `info` is lifecycle, `debug` is development.
- **Never log** media bytes, raw IP addresses, result tokens, or exact coordinates when the job is in public mode.

## 5. Numerics and DSP

The results go into a thesis, so the numbers must be reproducible and the units must not be confused.

| Rule | Detail |
| --- | --- |
| Units in types | Newtypes for `Hertz`, `Seconds`, `Metres`, `MetresPerSecond`, `Radians`, and for latitude and longitude. Degrees appear only at the I/O edge. |
| Two clocks | Audio time (`t_a`, seconds from clip start) and UTC time (`t_u`, milliseconds) are different types. Conversion happens in one function that takes the offset `s` (SDD 4). |
| Precision | `f64` for all estimation, fitting and geodesy. `f32` only for stored sample buffers and the track store. Accumulate sums in `f64`. |
| Non-finite values | Reject NaN and infinity at every boundary. Compare floats with `total_cmp` or an explicit tolerance, never `==`. |
| Constants | Speed of sound comes from the atmosphere model, never a literal. Earth model and constants live in `geodesy` once. |
| Determinism | Seeded `rand_chacha` generators only. No `HashMap` iteration in anything that affects a result: use `BTreeMap` or sorted vectors. Parallel reductions use fixed chunking and an ordered combine, so results match across thread counts. |
| Allocation | No allocation inside per-sample or per-bin loops. Reuse FFT plans and scratch buffers. |
| Performance | Measure with `criterion` before optimising. No hand-written SIMD and no `unsafe`; rely on the compiler and on `rayon`. |
| Provisional numbers | Any threshold not yet fixed by a gate is a named config value tagged `PROVISIONAL(G#)`, never a bare literal. |

## 6. Concurrency and async

- **Async only at the edge.** `tokio` appears only in `runtime`, `api` and `worker`. `runtime` is the one library crate allowed async and tokio; all other library crates contain no `async fn`, no `tokio` dependency, and no `std::thread::sleep`.
- **CPU work off the executor.** DSP and search run through `spawn_blocking` into the `rayon` pool (SDD 11.1). No loop over samples, file read or database call that blocks inside an async task.
- **No lock across `.await`.** Share state through channels and the database. If a mutex is unavoidable, keep the guard in a scope with no await inside it.
- **Bounded everything.** Channels, queues, request bodies and job wall time have explicit limits (SDD 11.3). An unbounded channel needs a proposal.
- **Cancellation.** Long stages check a cancellation flag at stage boundaries and inside long loops, and return a typed error. A cancelled job still deletes its media.
- **Timeouts** on every outbound call and every external subprocess.
- **Shutdown.** On SIGTERM stop accepting jobs, let running jobs finish or hit their limit, release leases, then exit. Task ownership, cancellation and outcome mapping follow SDD section 11.6: no bare `tokio::spawn` (denied in `clippy.toml`), and every task lives in a `Scope`.

## 7. Types, APIs and crate boundaries

- **Make wrong states impossible.** Use enums that carry their data (`Verdict::Locatable { region, mass }`) instead of a status string plus optional fields. Keep invariants private and expose constructors that validate.
- **Traits only at seams.** The seams are `TrackSource`, `Channel`, `Clock` and `MediaDecoder`. Everything else is concrete types and functions. Do not add a trait for one implementation.
- **Time is injected.** Code that needs the current time takes a `Clock`. Tests use a fake one.
- **Wire types are separate.** Request and response structs live in `api`. Domain types in the library crates do not derive `Serialize` for the network, and the public-mode result type has no field for exact coordinates.
- **No global mutable state.** No `static mut`, no `lazy_static` configuration. Configuration is one validated struct built at startup and passed down.
- **SQL.** Parameterised `sqlx` queries only, checked at compile time with committed offline data. Queries stay in `store`. Use the common SQL subset so migrations run on SQLite and Postgres (SDD 9.6).
- **Public surface stays small.** Items are private by default. Anything `pub` in a library crate is part of its contract and is documented.

## 8. Testing

The levels and the gate-to-test table are in SDD section 13. These rules govern how the tests are written.

- **Every requirement has a test.** A pull request that implements FR-xx adds or updates a test that names it. CI checks `docs/traceability.csv`.
- **Names** read as `unit_condition_expected`, for example `retarded_time_straight_pass_matches_root_finder`.
- **Numerical tests state tolerances** as relative plus absolute, with a reason. No unexplained `1e-6`.
- **Seeded and offline.** Tests use fixed seeds, the fake clock, and local fixtures. No network, no wall-clock time, no dependence on thread count.
- **Small fixtures.** Test data under 1 MB per file, in `testdata/`. Larger data is generated by `sim` on the fly.
- **Property tests** for invariants (transform round trips, whitening, monotonic likelihood at the truth) with the seed printed on failure.
- **Regression first.** A fixed bug gets a failing test before the fix.
- **Never weaken a test to pass.** Changing an expected value needs the reason in the pull request. Coverage is a signal, not a target.

## 9. Dependencies and licensing

Fewer dependencies means fewer risks. The following are approved for their stated job; versions are the latest stable when first added, then fixed by `Cargo.lock`.

| Job | Crate |
| --- | --- |
| Async runtime, HTTP and cancellation | `tokio`, `tokio-util` (for `CancellationToken`), `axum`, `tower`, `tower-http` |
| Outbound HTTP (ADS-B sources) | `reqwest` with rustls and the platform's native root certificates; the bundled `webpki-roots` is MPL-2.0 and is not allowed |
| Serialization and configuration | `serde`, `serde_json`, `toml` |
| Database | `sqlx` (SQLite now, Postgres later) |
| Errors and logging | `thiserror`, `anyhow` (binaries only), `tracing`, `tracing-subscriber` |
| Metrics (FR-OP-03) | `metrics`, `metrics-exporter-prometheus` |
| Parallelism | `rayon` |
| FFT and linear algebra | `rustfft`, `realfft`, `nalgebra` |
| Nonlinear least squares | `levenberg-marquardt`, or a small in-house version in `events` if the crate lags `nalgebra` |
| Statistics (chi-square thresholds) | `statrs` |
| Columnar data and compression | `arrow`, `parquet`, `flate2` (gzipped archive dumps) |
| Hashing and keyed hashes | `sha2` (manifests), `hmac` (keyed IP hash), `subtle` (constant-time compare) |
| Figures (FR-OUT-02) | `plotters` with the SVG backend |
| Randomness | `rand`, `rand_chacha`, `rand_distr` |
| Tests and benches | `proptest`, `criterion`, `tempfile` (dev-dependencies only) |
| CLI, ids, time | `clap`, `uuid` (v7), `time` |
| WAV | `hound` |

Anything else needs an entry in `docs/third_party.md` first: name, version, licence, what it replaces, and why the standard library or an approved crate is not enough.

- **Licences** allowed: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode. No GPL, LGPL or AGPL dependencies, a policy choice that keeps dual licensing possible later. `cargo deny` enforces the list.
- **No git or path dependencies** outside the workspace.
- **Audio decoding** by an external `ffmpeg` process is a subprocess, never linked, which keeps its licence out of our binary (SDD T-1).
- **Project licence** is AGPL-3.0-only, chosen by the author. The LICENSE file at the repository root is the single source of truth, and the license field in every Cargo.toml matches it. All allowed dependency licences above are compatible with it.

## 10. Security and privacy in code

These turn SDD section 12 into code rules.

- **Media.** Uploaded media is written only to the tmpfs scratch directory and deleted in a guard that runs on success, failure and cancellation. The sweeper is the backstop, not the plan.
- **IP addresses.** The raw address exists only inside the rate-limit middleware. It is hashed there with the keyed daily salt, and only the hash is passed on or stored.
- **One place for coarsening.** A single function in `pipeline` produces public result geometry. No other code builds a public location, and the public result type cannot hold exact coordinates.
- **Limits before work.** Check size, duration, rate and attestation before reading or decoding the body.
- **Decoder isolation.** Decoding runs in a child process with CPU, memory and time limits. The parent treats its output as untrusted and validates sample rate, length and finiteness.
- **URL fetching.** Allow-listed hosts only, private and loopback ranges blocked after DNS resolution, redirects off the list refused, size and time capped.
- **Tokens.** Result tokens come from the operating system random generator, 128 bits, compared in constant time.
- **Secrets** come from environment variables only, are never committed, and have redacted `Debug` output.
- **Dependencies** are scanned in CI (`cargo audit`, `cargo deny`). A new advisory blocks merging until handled or documented.

## 11. Agent workflow, git and review

**Build order.** Follow the simulator-first plan so every stage can be checked against known truth:

1. `core-types`, `geodesy`, `acoustics`
2. `sim`, including the forward model and the Cramér–Rao bound
3. `dsp`, then `events`
4. `adsb`, then `search`
5. `pipeline`, then `store`, then `runtime` (after both)
6. `api`, `worker`, `cli`

**Working rules**

- One task per branch, one concern per pull request, ideally under 400 changed lines. Commit messages follow Conventional Commits (`feat(dsp): ...`).
- The pull request description lists the requirement ids, the commands run with their result, and known gaps. It does not claim anything that was not run.
- Do not disable, skip or loosen a lint, test or CI step to get green. Fix the cause or record a question.
- Do not invent numbers. Thresholds, limits and error ranges come from config or from measured data, and unfinished ones carry `PROVISIONAL(G#)`.
- Do not commit generated data, audio or secrets. Large fixtures are produced by `sim`.
- A change that touches the data model adds a migration and updates SDD section 9 through a proposal.
- When blocked, record the question in `docs/questions.md`, take the SDD default, and continue with the next task.

## 12. Definition of done

A task is done only when every item is true.

- [ ] Format, clippy (warnings denied), tests, `cargo deny` and `cargo audit` all pass with `--locked`
- [ ] A test names each requirement id the task implements
- [ ] Public items are documented, and formulas cite their SDD section and units
- [ ] No `unwrap`, `expect`, `panic!` or unchecked indexing in library code
- [ ] No new dependency without an entry in `docs/third_party.md`
- [ ] Results are deterministic across thread counts for a fixed seed
- [ ] Nothing logs media, raw IPs, tokens or exact public coordinates
- [ ] The pull request lists what was run and what is still open
- [ ] Open questions are in `docs/questions.md`, and unfinished thresholds are tagged `PROVISIONAL`
