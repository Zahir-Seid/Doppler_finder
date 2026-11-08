# Track model: interpolation, legs, closest approach

ADSB-03 turns stored [`FlightTrack`]s into the quantities the localization
stages consume (SDD sections 4 and 6.1). All of it is deterministic: the same
track always produces the same interpolant, legs and closest approach.

## Hermite interpolation

`adsb::TrackInterpolator` evaluates a track at arbitrary times. Positions are
interpolated as cubic Hermite splines in a local East-North-Up frame anchored
at the first sample, so longitude wraps and latitude distortion never enter
the cubic. Knot velocities are centred finite differences of the stored
positions; the optional reported velocity fields are not needed.

The interpolant passes exactly through every sample and has continuous
position and velocity. It requires at least two samples with strictly
increasing timestamps; a repeated timestamp is rejected because a cubic
cannot span it.

Two entry points share the interpolant:

- `interpolate(track, at)` returns a [`TrackSample`] flagged
  `FLAG_INTERPOLATED`. Barometric altitude, vertical rate, ground speed and
  track angle keep the previous linear semantics.
- `TrackInterpolator::position_ecef(unix_time_s)` returns the position in
  ECEF, the seam `acoustics::solve_emission_time` plugs into when matching
  events against a track.

## Legs

`adsb::Leg` is the derived `legs` table row of SDD 9.2, built by
`Leg::from_track` or `legs_from_tracks`:

- `leg_id` is deterministic: `<source>:<icao24>:<start_ms>`, so replaying a
  store produces identical ids.
- Time span, bounding box and point count come straight from the samples.
- `median_gap_s` and `max_gap_s` are measured from the positive gaps actually
  present; a single-sample leg reports `0.0`.
- `engine_class` classifies the ICAO type code as `Propeller`, `Jet` or
  `Unknown`. Turboprops count as `Propeller` because they emit the tone the
  detector listens for. The tables in `legs.rs` are a starter set, not the
  full ICAO catalogue — unlisted codes stay `Unknown`, which the silence term
  treats probabilistically (SDD 6.2, Q-15).

One leg is derived per track. Splitting on long reporting gaps is not done
yet because no gap threshold is fixed; `max_gap_s` exposes the worst gap so a
later stage can decide (Q-15).

## Closest approach

`adsb::closest_approach(track, observer, observer_height_m,
sound_speed_m_per_s)` implements the SDD 6.1 prediction for one observer and
one flight. It minimizes the 3-D slant range over the Hermite interpolant
with a golden-section search on every interpolation interval, then applies
the light-time relation `t_r = t_e + range / c` exactly — no iteration is
needed because the emission time is already known; the bracketed solver in
`acoustics` covers the inverse direction used when matching events.

`observer_height_m` is the ellipsoidal height of the listener, as returned by
`geodesy::observer_height_m` (terrain plus ear height plus geoid undulation).
`geometric_altitude_m` on samples is used as the ellipsoidal height.

The result carries the emission time of minimum range, the heard time, the
slant range and the horizontal speed over ground at closest approach.

## Region support flags

`adsb::assess_coverage` implements the T-4 flag half of ADSB-04: it reduces
an ingest result to `Supported`, `Insufficient` or `Unsupported` from the
measured sampling gaps, so an empty track set is never silently treated as a
complete one. `Unsupported` (empty ingest) maps to `no_adsb_coverage` and
`Insufficient` (median gap above `MAX_SUPPORTED_MEDIAN_GAP_S`, provisional at
10 s pending T-4 measurement) maps to `region_unsupported` downstream; the
code mapping lives in `api`.
