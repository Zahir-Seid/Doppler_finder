# Simulator scenarios

Scenario files are JSON serializations of `sim::Scenario`. They are the ground
truth for estimator tests and the G1/G2 studies. Every scenario carries a seed;
a run is not reproducible without it.

`example-single-pass.json` is a format example, not a calibration case. Real
scenario suites are added with the simulator implementation and their expected
metrics live beside them in `testdata/`.
