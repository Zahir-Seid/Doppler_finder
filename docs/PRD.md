# PRD — Doppler Locatability Audit

Nov 4, 2026 · @zahir

## 1. Overview

Doppler Locatability Audit tells a person how well a recording can be placed on a map from the faint Doppler tone of propeller aircraft passing overhead, matched against public flight records. The promise: upload a recording and get an honest answer, either a region with a stated confidence or "not locatable", plus the evidence and uncertainty behind it.

**Problem.** A viral 2026 video showed that inaudible background tones from aircraft can place a recording to roughly 100 m, sometimes from under a minute of audio. Its method was manual (spectrogram tracing, hand-fitted curves, fixed grids) and gave no uncertainty estimate. Ordinary people cannot check whether their own footage leaks its location, and the method has no rigorous error analysis.

**This product** has two faces that share one core:

- A research-grade estimator with quantified uncertainty, which is the basis of the thesis.
- A public demo that works as a defensive audit: "how locatable is my recording, and which clues leak it?"

Working title: Doppler Locatability Audit. Core is Rust (Axum service, systems-level DSP and search); the frontend is Nuxt with bun, specified separately in `ui_ux.md`.

## 2. Goals and non-goals

**Goals**

- G1. Automatically measure aircraft Doppler events in a recording, with per-parameter uncertainty and no manual tracing.
- G2. Produce a location estimate whose stated confidence region is calibrated: it contains the truth about as often as it claims.
- G3. Quantify which error source (ADS-B timing, altitude datum, terrain, wind, propeller RPM drift, audio SNR) limits accuracy, using a forward simulator.
- G4. Refuse rather than guess: one flyover, or inconsistent evidence, returns "not locatable".
- G5. Ship a public demo that is useful as a privacy self-check and hard to misuse.
- G6. Make every thesis number reproducible from a stored run manifest.

**Non-goals**

- Tracking or identifying people, or a service for locating other people's footage.
- Real-time or streaming localization.
- Hosting or sharing uploaded audio.
- Accounts, billing or history in v1.
- Non-aircraft sound sources (bells, hum) in v1; those are v3 experiments.
- A mobile app.

## 3. Users and use cases

| Persona | Need | Release |
| --- | --- | --- |
| Researcher (thesis author, examiner) | Run the estimator on simulated and real data, inspect uncertainty, reproduce every result from a manifest | v0 |
| Privacy-conscious creator | Upload own footage, learn whether it leaks location and why, get advice on what removes the leak | v1 |
| Curious public visitor | Try the demo on a sample clip and understand the method | v1 |
| Partner developer (emergency or aid use, Ethiopia) | Consent-based locating where a person wants to be found; exploratory, no commitment | v3 |

## 4. Release scope

| Release | Includes | Exit gate |
| --- | --- | --- |
| v0 Research core | Simulator, error-budget study, single-event estimator, two-event scoring on open ADS-B data, CLI and library, run manifests | G1, G2 (section 8) |
| v1 Public demo | Axum API, job queue, Nuxt UI, anonymous use with rate limits, report page, coarse output by default | G3, G4, G5 |
| v2 Accounts | Sign-in, history, quotas, API keys, research mode with precise output for invited users | Product review |
| v3 Experiments | Grid hum as an absolute-time channel, ground-reflection height, bell beacons, Ethiopia case study | Each experiment's own gate |

Phase order follows the thesis plan: the error budget decides what v1 is worth building.

## 5. Functional requirements

Priority: M = must, S = should, C = could. Phase: release in which it first ships. Every requirement gets an acceptance test ID in the validation plan.

| ID | Requirement | Pri | Phase |
| --- | --- | --- | --- |
| FR-IN-01 | Accept audio or video upload (m4a, mp3, wav, mp4, webm), decoded server-side; reject unsupported, oversize or overlong files with a clear error | M | v1 |
| FR-IN-02 | User gives local date or UTC window and a coarse region (US state or bounding box) | M | v0 |
| FR-IN-03 | Optional known recording time (UTC) tightens the search | S | v0 |
| FR-DM-01 | Detect candidate propeller flyovers automatically, with no manual tracing | M | v0 |
| FR-DM-02 | Per event, estimate fundamental, speed, closest distance and heard closest-approach time, each with covariance | M | v0 |
| FR-DM-03 | Fit uses the emission-time (retarded) Doppler model, not a delayed copy of the curve | M | v0 |
| FR-DM-04 | Joint fit across visible harmonics | S | v0 |
| FR-DM-05 | Direct parametric estimator on the audio for faint events, with track-then-fit as the fast path | S | v0 |
| FR-DM-06 | Per-event quality score; low-SNR events are down-weighted or dropped, and the report says so | M | v0 |
| FR-DA-01 | Ingest ADS-B tracks for the date and region from open sources; record source, sampling interval and latency in the run manifest | M | v0 |
| FR-DA-02 | Convert altitude to geometric height with a geoid model; take terrain from a public DEM | M | v0 |
| FR-DA-03 | Use measured wind and temperature for speed of sound; fall back to a standard atmosphere and flag it | S | v0 |
| FR-DA-04 | Cache ingest products so reruns work offline | S | v0 |
| FR-SL-01 | Score candidates with soft likelihoods (Doppler fit, inter-event time offset, silence), not hard filters | M | v0 |
| FR-SL-02 | Refuse with "not locatable" for one event and no known time, or for inconsistent evidence | M | v0 |
| FR-SL-03 | Refine to a continuous position with a 2-D covariance ellipse | M | v0 |
| FR-SL-04 | Reported confidence regions meet the calibration gate (section 8) | M | v0 |
| FR-SL-05 | Report ranked alternatives and the margin over the best rival | S | v0 |
| FR-OUT-01 | Verdict is one of Locatable, Ambiguous, Not locatable, with the reason | M | v1 |
| FR-OUT-02 | Report shows spectrogram with fitted curves, matched flights, uncertainty and per-channel evidence | M | v1 |
| FR-OUT-03 | Public demo returns coarse output by default (decision D-1) | M | v1 |
| FR-OUT-04 | Export result JSON and the run manifest | M | v0 |
| FR-SIM-01 | Forward simulator generates audio and ground truth from a location, trajectory and error settings | M | v0 |
| FR-SIM-02 | Error-injection suite with a ranked sensitivity report | M | v0 |
| FR-SIM-03 | Raw-audio Cramér–Rao bound for any simulated scenario | S | v0 |
| FR-SIM-04 | Golden fixtures and regression tests in CI | M | v0 |
| FR-AC-01 | No accounts in v1; anonymous cookie session | M | v1 |
| FR-AC-02 | Limit of 10 jobs per 24 h per IP and 10 per session, whichever is hit first, plus a global daily compute budget and concurrency cap | M | v1 |
| FR-AC-03 | Bot challenge on upload | S | v1 |
| FR-AC-04 | Uploaded media deleted within 1 h of job end; results kept 24 h under an unguessable ID | M | v1 |
| FR-AC-05 | Attestation that the user owns or may analyse the recording | S | v1 |
| FR-OP-01 | Job queue with progress over server-sent events, and cancel | M | v1 |
| FR-OP-02 | Structured logs that never contain audio or precise coordinates | M | v1 |
| FR-OP-03 | Health and metrics endpoints | S | v1 |
| FR-EX-01 | Grid-hum channel supplies an absolute-time likelihood | C | v3 |
| FR-EX-02 | Ground-reflection channel supplies a listener-height constraint | C | v3 |

## 6. Non-functional requirements

Targets marked provisional are set after the v0 error-budget study.

| ID | Requirement | Target |
| --- | --- | --- |
| NFR-01 | Honest uncertainty: the 95% region contains the truth at about the stated rate | At least 90% coverage on simulation and on real clips (provisional) |
| NFR-02 | Latency for a typical research clip (10 min, one state, one day) on one 8-core node; the public demo caps clips lower (SDD section 11.3) | Under 5 min, with live progress (provisional) |
| NFR-03 | Cost: single node, with a per-job compute cap | Cap set from v0 profiling |
| NFR-04 | Reproducibility: any result can be regenerated from its manifest (data versions, parameters, seed) | 100% of reported results |
| NFR-05 | Toolchain: stable Rust, Axum on tokio, mainstream crates | No nightly features |
| NFR-06 | Licence hygiene: every third-party code and data source is listed with its licence; code under restrictive terms is excluded unless the owner documents permission | Checked in CI |
| NFR-07 | Testability: DSP and physics covered by golden tests against the simulator | Every FR has a test |
| NFR-08 | Upload safety: decoding runs in a restricted process with size, time and memory limits | Enforced in the SDD |

## 7. Privacy, ethics and abuse controls

The main risk is misuse: locating a private person from footage they posted. The demo is therefore framed and built as a self-audit, and its strongest safeguard is limiting output precision, not limiting tries.

- **Precision.** The public demo returns a coarse area and a confidence level by default; precise coordinates are available only in local or invited research mode (decision D-1).
- **No retention.** Media is deleted within 1 h of job end, results expire after 24 h, and logs hold no audio or precise coordinates.
- **Attestation.** The user confirms they own the recording or may analyse it. This is friction and a record of intent, not verification.
- **Rate limits.** 10 jobs per IP and 10 per session per 24 h, plus a global daily compute budget and a bot challenge. Both limits are bypassable with a VPN or cleared cookies. Carrier-grade NAT and campus networks also put many legitimate users behind one IP, so the per-IP limit can block real people. Treat limits as cost control and friction.
- **Defensive guidance.** Reports explain what leaks (which aircraft events, which channels) and what helps: cutting or replacing the audio. The source video reports that high-pass filtering, masking noise and phone-call compression do not remove the signal.
- **Takedown and contact.** The site lists a contact address and a deletion route.
- **Thesis.** A threat-model chapter states the capability, the misuse paths and the mitigations, and the code for a precise public locator is not released.

## 8. Success metrics and validation gates

Thresholds are provisional and are fixed once the v0 error budget exists.

| Gate | Measure | Provisional threshold | Phase |
| --- | --- | --- | --- |
| G1 | Simulator recovery: share of 1,000 simulated scenarios whose 95% region contains the truth | At least 90% | v0 |
| G2 | Error-budget report: every injected source ranked by its contribution to position error | Complete and reviewed | v0 |
| G3 | Real-clip calibration: clips with known location, US first, any Addis clips second | At least 10 clips; coverage at least 90% | v1 |
| G4 | Refusal: single-event clips without a known time are refused | 100%; confident wrong answers at most 5% of answered clips | v1 |
| G5 | Demo robustness: 10 concurrent jobs on one node | No failures, no leaked media | v1 |
| G6 | Thesis reproducibility: results chapters regenerate from manifests | 100% | v1 |

The published location of the source video's reference recording is used only as a late validation case, after the pipeline is built independently.

## 9. Data sources, constraints and assumptions

No own receiver: the receiver was too costly, so all flight data comes from open archives. Sampling rate, latency and coverage of these archives are unverified and are measured in v0.

| Source | Use | Terms | Caveat |
| --- | --- | --- | --- |
| [OpenSky Network (Trino)](https://openskynetwork.github.io/opensky-api/trino.html) | Primary ADS-B history | Free for academic and government research after an access request | Queries need a partition filter; some tables keep about a year only; commercial use needs a licence |
| [adsb.lol history](https://www.adsb.lol/docs/open-data/historical/) | Fallback ADS-B history and the US reproduction case | ODbL 1.0, daily dumps with one JSON file per aircraft | Repos are archived per year; sampling is adaptive and must be measured |
| [ADS-B Exchange](https://www.adsbexchange.com/?p=39137) | Development samples only | Paid; free first-of-month samples | Commercial use needs a licence |
| Flightradar24 | Not used | Paid API; scraping is excluded | Terms-of-service risk |
| Terrain and geoid | Slant distance and height | Public DEM and geoid grids, chosen in the SDD | Vertical error and tree canopy |
| [Dove's Guide](https://dove.cccbr.org.uk/help/searching) | v3 bell experiment | CSV under CC BY-SA 4.0 | Lists nominal notes, not measured pitch |

Assumptions to verify in v0:

- A1. Open archives sample fast enough that interpolation error does not dominate the budget.
- A2. A usable recording holds at least two propeller flyovers, or one flyover plus a known time.
- A3. ADS-B coverage near Addis is sparse, so Ethiopia is a stretch case until measured.
- A4. Propeller traffic exists near the validation clips.

Links above come from search results and are to be re-checked before the thesis cites them.

## 10. Risks and open decisions

| ID | Decision | Recommendation | Needed by |
| --- | --- | --- | --- |
| D-1 | Public demo output precision | Coarse area by default; precise only in local or invited research mode | Before v1 |
| D-2 | Base the code on the existing fork or on a clean workspace | Clean workspace; the fork's licence adds a restriction on Anthropic access and its runtime is not tokio | Before SDD freeze |
| D-3 | Product name | Keep the working title until the thesis title is set | Before v1 |
| D-4 | Job and result store | SQLite in v1, Postgres when accounts arrive | SDD section 9 |
| D-5 | Hosting and monthly budget | One VM sized from v0 profiling | Before v1 |

| ID | Risk | Mitigation |
| --- | --- | --- |
| R1 | ADS-B timing error dominates accuracy | Error-budget study first; report honest ellipses |
| R2 | Open archives sample too slowly | Measure in v0; document the limit in the thesis |
| R3 | No ADS-B coverage near Addis | Check coverage early; US stays primary |
| R4 | Demo misuse | Coarse output, attestation, limits, no retention |
| R5 | State-wide search is too slow or costly | Coarse-to-fine search, per-job compute cap |
| R6 | An open-source implementation of the video's method already exists | Differentiate on error budget, calibration, honest uncertainty and the defensive framing |

## 11. Document map and traceability

| Document | Purpose | Status |
| --- | --- | --- |
| PRD (this) | What and why; requirement and gate IDs | Draft |
| SDD | Methods (signal model, estimators, likelihoods, error budget), data design, crates, job model, API, concurrency, security, deployment. It replaces the separate method spec and data design documents | Drafted |
| Rust Working Norms | Toolchain, lints, error mapping, test rules and definition of done for agent-written code | Drafted |
| [Traceability](traceability.csv) | Requirement to section to test to status | Living |
| [Validation plan](validation-plan.md) | Datasets, gates, acceptance tests per FR | Drafted |
| [UI and UX](ui_ux.md) | Nuxt and bun screens, states and flows | Draft, before v1 |
| [Questions](questions.md) | Open questions and defaults | Living |
| [Third-party inventory](third_party.md) | Code and data licences | Living |

Traceability rule: every FR ID appears in one SDD method section, one SDD component and one test ID. Code and tests cite the FR ID they satisfy, so a smaller model can implement one requirement at a time.
