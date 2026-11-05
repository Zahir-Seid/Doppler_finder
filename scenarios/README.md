# Simulator scenarios

Scenario files are JSON serializations of `sim::Scenario`. They are the ground
truth for estimator tests and the G1/G2 studies. Every scenario carries a seed;
a run is not reproducible without it.

A pass is straight and level. It states its speed, its horizontal miss distance
(`closest_approach_m`), the offset of closest approach from the start of the
recording (`offset_s`), its geometric altitude, its track direction
(`heading_deg`, clockwise from north, default 90°), and its propeller tone
(`fundamental_hz` and relative `harmonics`, defaulting to 100 Hz and the
fundamental alone). The slant range at closest approach is
`hypot(closest_approach_m, altitude_m - ear_height_m)`; the audio does not
depend on the heading, but the degraded feed and the position bound do.

## Outputs of one scenario and one seed

- `sim::synthesize` — mono samples on the audio clock, with optional air
  absorption (`SynthesisConfig::absorption`) and seeded noise
  (`SynthesisConfig::noise`).
- `sim::write_wav` — the same samples as 16-bit PCM.
- `sim::truth` and `sim::write_truth` — per pass, the emitted and heard closest
  approach, the propagation delay, the slant range and the tone parameters.
- `sim::degraded_feed` — the synthetic ADS-B feed under an `ErrorProfile`.
- `sim::event_bound` and `sim::position_bound` — what noise alone allows.

## Fixtures

`example-single-pass.json` is a format example, not a calibration case. Its
synthesized audio and truth values are pinned by
`crates/sim/tests/golden.rs`; changing the scenario requires regenerating that
digest in the same commit and saying why. Larger scenario suites are added with
the simulator's study harness, and their expected metrics live beside them in
`testdata/`.
