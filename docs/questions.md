# Open questions

Each entry records an ambiguity, the default used until it is resolved, and who
closes it. SDD section 14 holds the technical decisions T-1 to T-7; this file
holds implementation-level questions and cross-document risks.

## Q-1: Public clip cap versus the two-flyover assumption

- **Status:** Open, v0 measurement required.
- **Context:** SDD 11.3 caps public-demo clips at 25 MB / 90 s. Assumption A2
  needs at least two flyovers for a point fix. A 90 s clip may rarely contain
  two.
- **Default:** Keep 90 s provisional for the demo; local CLI accepts up to
  600 s. Do not promise a point fix from one event.
- **Resolution:** Count how many real clips contain two qualifying events in
  the v0 dataset, then set the final demo cap. Record the number in the
  validation plan.

## Q-2: Mirror ambiguity of a single pass

- **Status:** Resolved in text, test still required.
- **Context:** A single straight pass is mirror-symmetric about the track;
  silence and land priors may not always break the tie.
- **Default:** Keep both mirror positions in the posterior. The verdict is at
  most `Ambiguous` unless a second event with different geometry, the silence
  term or the land prior removes one. Never report `Locatable` from a
  mirror-degenerate posterior.
- **Resolution:** Add a simulator regression with an exact mirror pair and
  assert `Ambiguous` or `NotLocatable`.

## Q-3: Approved crate list is still short

- **Status:** Open, add per task.
- **Context:** The SDD implies crates the norms table does not list: `toml`
  for `config/default.toml`, `sha2` for manifest hashes, `hmac` + `subtle` for
  keyed IP hashes and constant-time token comparison, an image/plot crate for
  FR-OUT-02 figures, `flate2` if ADS-B archives are compressed, and a metrics
  crate for FR-OP-03.
- **Default:** Do not add a crate before its licence and purpose are recorded
  in `third_party.md`. Prefer crates already allowed by `deny.toml`.
- **Resolution:** Extend the approved table in the norms through a proposal,
  then update `deny.toml`.

## Q-4: TLS root store for `reqwest`

- **Status:** Open.
- **Context:** `reqwest`'s common `rustls-tls` feature pulls `webpki-roots`
  (MPL-2.0), which is outside the current licence allow-list.
- **Default:** Use `rustls-tls-native-roots` when the ingest client is added,
  or record a scoped exception in `deny.toml`.
- **Resolution:** Confirm the chosen feature set in `deny.toml` and
  `third_party.md`.

## Q-5: Public wording versus the locatable threshold

- **Status:** Open, product copy.
- **Context:** A `Locatable` verdict requires a small covariance ellipse, while
  public mode reports a cell of 2 km or coarser. The two are not contradictory
  but the copy must not imply metre-level public accuracy.
- **Default:** Public copy says "region" and the stated confidence, never
  "exact location" unless research mode is enabled.
- **Resolution:** Approve the result-page copy in `ui_ux.md`.

## Q-6: Frontend specification

- **Status:** Draft, blocks v1.
- **Context:** The PRD promises a Nuxt/bun frontend and the SDD defers it to
  `ui_ux.md`.
- **Default:** No frontend work until the page states, copy and privacy rules
  in `ui_ux.md` are approved.
- **Resolution:** Author review, then scaffold `web/`.

## Q-7: `Scope` ownership

- **Status:** Resolved.
- **Context:** Earlier drafts placed `Scope` in both binaries.
- **Decision:** `Scope` lives once in `runtime`. `api` and `worker` depend on
  `runtime`; neither binary depends on the other.

## Q-8: Wall-time limit for research runs

- **Status:** Resolved.
- **Context:** SDD 11.3 previously applied a 120 s limit to every job.
- **Decision:** 120 s applies to service jobs; local CLI research runs use the
  NFR-02 budget of 300 s for a typical 10-minute clip. Encoded in
  `pipeline::RunMode`.

## Q-10: Wind sign and path averaging

- **Status:** Open until the weather source is chosen (v1).
- **Context:** SDD section 4 says wind enters as the component along the
  propagation path, averaged along the path from public weather data, but it
  fixes neither the sign nor how the average is formed.
- **Default:** `Atmosphere::effective_speed_m_per_s` takes one component,
  positive when the wind blows from the source towards the observer, so it
  shortens the propagation time. Path averaging waits for the weather source;
  the simulator injects the component directly.
- **Resolution:** Fix the sign in the weather adapter's documentation and
  validate the averaging against the simulator's wind scenario (SDD 7).

## Q-11: Noise colour and SNR convention

- **Status:** Open until G2.
- **Context:** SDD section 7 asks for noise at a target SNR with a stated
  colour, but fixes neither the SNR definition nor the colour's slope.
- **Default:** `NoiseConfig::snr_db` is peak-to-RMS: the noise RMS is the
  clean peak divided by `10^(snr/20)`, measured after every pass is mixed and
  before noise is added. `NoiseColour::Pink` is three one-pole low-pass
  sections at 60, 150 and 400 Hz, summed and rescaled, which tilts energy low
  without claiming a 1/f slope.
- **Resolution:** Measure the noise floor and spectrum of the real archives,
  then either keep the convention or restate it in `sim`'s documentation and
  regenerate the golden digest.

## Q-12: Air-absorption coefficients

- **Status:** Open until G2.
- **Context:** SDD section 7 asks for "a simple absorption term" without a
  model or coefficients, and absorption at 100–300 Hz over a few kilometres is
  small compared with spreading.
- **Default:** `AirAbsorption::new(db_per_km_at_1_khz)` scales the coefficient
  by `(f / 1 kHz)^1.5` and applies it per harmonic over the slant range. The
  default is `AirAbsorption::none()`, so synthesis stays lossless until a
  measured coefficient exists.
- **Resolution:** Fit the shape and the coefficient to ISO 9613-1 at the
  measured temperature and humidity, then re-derive the golden digest and say
  in the error budget whether absorption matters at all.

## Q-14: Analysis band and rate

- **Status:** Open.
- **Context:** SDD section 5 fixes a passband of about 20–1500 Hz and an 80 dB
  stopband, but not the output rate, the exact lower edge or the width of the
  transition band. `docs/validation-plan.md` also says the 20 Hz high-pass
  should be checked against real-device microphone roll-off before it is fixed.
- **Default:** Resample to 4 kHz with a 20 Hz lower edge, a 1.5 kHz passband
  edge, a 1.9 kHz stopband edge and 80 dB attenuation. The high-pass stopband
  starts at one quarter of its passband edge. The 4 kHz rate gives 800 Hz of
  margin above the passband and keeps later stages cheap.
- **Resolution:** Measure detection and fit quality against the G1/G2 scenario
  suite and real clips, including phone low-frequency roll-off; change the
  default only with the measurement recorded.

## Q-13: Error-budget ranking needs the estimators

- **Status:** Open until `events` and `search` exist.
- **Context:** FR-SIM-02 asks for a ranked sensitivity report, and G2 asks
  which source limits position accuracy. `inject::sensitivity` ranks the
  sources by how far each moves the closest distance, the heard time and the
  speed of the track, which is input error, not position error.
- **Default:** Ship the feed and the input-error ranking now; mark FR-SIM-02
  partial and do not quote its ranking as the G2 error budget.
- **Resolution:** Rerun the one-at-a-time and combined profiles through the
  real estimators, decompose the position variance across the combined draws,
  and replace the ranking with that decomposition.

## Q-9: Licence

- **Status:** Resolved.
- **Decision:** The project is AGPL-3.0-only. The root licence is accepted.
  `deny.toml` allows only the listed permissive licences for dependencies;
  AGPL is the project's own licence, not a dependency licence.
