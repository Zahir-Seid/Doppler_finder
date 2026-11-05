//! Emission-time solver for real tracks (SDD section 4).
//!
//! The straight-pass closed form only seeds a fit. Matching against an
//! interpolated ADS-B track needs `t_e` from `t_r = t_e + ‖p(t_e) − x‖ / c`,
//! where `p` is the track and `x` the observer. The residual
//! `g(t) = t + range(t)/c − t_r` is strictly increasing while the aircraft is
//! subsonic, which is why a bracketed solve is well-posed (SDD section 4).

use core_types::Seconds;
use geodesy::Ecef;

use crate::doppler::validate_sound_speed;
use crate::error::AcousticsError;

/// Narrowest bracket treated as converged, in seconds. A nanosecond of
/// emission time moves a tone by far less than the fit's own residual.
const BRACKET_TOL_S: f64 = 1e-9;

/// First backward step while bracketing, in seconds.
const FIRST_STEP_S: f64 = 0.5;

/// Furthest look-back from the reception time, in seconds. Ninety seconds of
/// sound travel is roughly 30 km, beyond the range any microphone in the search
/// can hear an aircraft.
const MAX_LOOKBACK_S: f64 = 90.0;

/// Iteration cap. Bisection halves the bracket each step, so this is far more
/// than the 40 iterations the widest bracket needs.
const MAX_ITERATIONS: u32 = 128;

/// Solve for the emission time of a sound heard at `reception_time`.
///
/// `position_at` is the source position at an emission time, in the same ECEF
/// frame as `observer`; a Hermite-interpolated track plugs in here. The
/// equation is `t_r = t_e + ‖p(t_e) − x‖ / c` from SDD section 4, solved by
/// bracketing at `t_r` and walking back with doubling steps, then closing with
/// a secant step inside the bracket and bisection when the secant leaves it.
///
/// Returns [`AcousticsError::InvalidSoundSpeed`] for a non-positive `c`,
/// [`AcousticsError::NotFinite`] for a non-finite input or a track that returns
/// a non-finite position, and [`AcousticsError::NoRealSolution`] when no
/// emission time exists within [`MAX_LOOKBACK_S`] seconds of `t_r`.
pub fn solve_emission_time(
    position_at: impl Fn(Seconds) -> Ecef,
    observer: Ecef,
    sound_speed_m_per_s: f64,
    reception_time: Seconds,
) -> Result<Seconds, AcousticsError> {
    validate_sound_speed(sound_speed_m_per_s)?;
    if !reception_time.0.is_finite() || !observer.is_finite() {
        return Err(AcousticsError::NotFinite);
    }

    let residual = |emission_time: Seconds| -> Result<f64, AcousticsError> {
        let position = position_at(emission_time);
        if !position.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        Ok(
            emission_time.0 + position.distance_m(observer) / sound_speed_m_per_s
                - reception_time.0,
        )
    };

    // `g(t_r) = range(t_r)/c >= 0`, so the reception time is always a valid
    // upper bound; walk back until the residual becomes non-positive.
    let mut upper = reception_time;
    let mut g_upper = residual(upper)?;
    let mut step_s = FIRST_STEP_S;
    let mut lookback_s = step_s;
    let mut lower = Seconds(reception_time.0 - step_s);
    let mut g_lower = residual(lower)?;
    while g_lower > 0.0 {
        if lookback_s >= MAX_LOOKBACK_S {
            return Err(AcousticsError::NoRealSolution);
        }
        upper = lower;
        g_upper = g_lower;
        step_s *= 2.0;
        lookback_s += step_s;
        lower = Seconds(upper.0 - step_s);
        g_lower = residual(lower)?;
    }
    if g_upper < 0.0 {
        // Only reachable for a non-monotone residual, which a subsonic track
        // cannot produce (SDD section 4).
        return Err(AcousticsError::NoRealSolution);
    }

    for _ in 0..MAX_ITERATIONS {
        let width = upper.0 - lower.0;
        if width <= BRACKET_TOL_S {
            break;
        }
        let secant = upper.0 - g_upper * width / (g_upper - g_lower);
        let trial = if secant.is_finite() && secant > lower.0 && secant < upper.0 {
            Seconds(secant)
        } else {
            Seconds(f64::midpoint(lower.0, upper.0))
        };
        let g_trial = residual(trial)?;
        if g_trial.abs() <= BRACKET_TOL_S {
            return Ok(trial);
        }
        if g_trial > 0.0 {
            upper = trial;
            g_upper = g_trial;
        } else {
            lower = trial;
            g_lower = g_trial;
        }
    }
    Ok(Seconds(f64::midpoint(lower.0, upper.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::Atmosphere;
    use crate::doppler::StraightPass;

    /// Absolute tolerance on a solved emission time, in seconds. The solver
    /// stops at a residual of 1 ns, and the closed form it is compared against
    /// is evaluated in the same precision, so 1 µs is a loose bound.
    const TIME_TOL_S: f64 = 1e-6;

    fn sound_speed() -> f64 {
        Atmosphere::standard().speed_of_sound_m_per_s()
    }

    /// A straight pass expressed as a position function, with the observer at
    /// the origin of this local metric frame. ECEF is used here as a Cartesian
    /// 3-vector; the physics only needs distances.
    struct PassTrack {
        closest_approach_m: f64,
        speed_m_per_s: f64,
        emitted_closest_approach_s: f64,
    }

    impl PassTrack {
        fn position_at(&self, emission_time: Seconds) -> Ecef {
            Ecef::new(
                self.closest_approach_m,
                self.speed_m_per_s * (emission_time.0 - self.emitted_closest_approach_s),
                0.0,
            )
        }
    }

    fn geometry() -> (StraightPass, PassTrack) {
        (
            StraightPass {
                closest_approach_m: 500.0,
                speed_m_per_s: 250.0,
            },
            PassTrack {
                closest_approach_m: 500.0,
                speed_m_per_s: 250.0,
                emitted_closest_approach_s: 100.0,
            },
        )
    }

    #[test]
    fn straight_pass_solver_matches_the_closed_form_root() {
        let c = sound_speed();
        let (pass, track) = geometry();
        // The closed form's offset is measured from the emitted closest
        // approach (SDD section 4), so the reception times are t0e + u.
        for received_offset_s in [-8.0, -1.0, 0.0, 1.0, 8.0] {
            let reception_time = Seconds(100.0 + received_offset_s);
            let solved = solve_emission_time(
                |emission_time| track.position_at(emission_time),
                Ecef::ORIGIN,
                c,
                reception_time,
            )
            .expect("solvable geometry");
            let expected = 100.0
                + pass
                    .retarded_offset(received_offset_s, c)
                    .expect("valid geometry")
                    .0;
            assert!(
                (solved.0 - expected).abs() < TIME_TOL_S,
                "u={received_offset_s}: solver {} s, closed form {expected} s",
                solved.0
            );
        }
    }

    #[test]
    fn solver_returns_an_emission_time_before_reception() {
        let c = sound_speed();
        let (_, track) = geometry();
        let reception_time = Seconds(130.0);
        let solved = solve_emission_time(
            |emission_time| track.position_at(emission_time),
            Ecef::ORIGIN,
            c,
            reception_time,
        )
        .expect("solvable geometry");
        assert!(
            solved.0 < reception_time.0,
            "sound must be emitted before it is heard, got {solved:?}"
        );
        assert!(
            reception_time.0 - solved.0 < MAX_LOOKBACK_S,
            "the delay must fit inside the look-back"
        );
    }

    #[test]
    fn solver_is_monotone_in_reception_time() {
        let c = sound_speed();
        let (_, track) = geometry();
        let mut previous = f64::NEG_INFINITY;
        for step in 0..8 {
            let reception_time = Seconds(100.0 + f64::from(step) * 0.5);
            let solved = solve_emission_time(
                |emission_time| track.position_at(emission_time),
                Ecef::ORIGIN,
                c,
                reception_time,
            )
            .expect("solvable geometry");
            assert!(
                solved.0 > previous,
                "emission time must grow with reception time, broke at step {step}"
            );
            previous = solved.0;
        }
    }

    #[test]
    fn solver_rejects_non_finite_inputs() {
        let observer = Ecef::ORIGIN;
        assert!(matches!(
            solve_emission_time(|t| Ecef::new(t.0, 0.0, 0.0), observer, 0.0, Seconds(1.0)),
            Err(AcousticsError::InvalidSoundSpeed(_))
        ));
        assert!(matches!(
            solve_emission_time(
                |t| Ecef::new(t.0, 0.0, 0.0),
                observer,
                340.0,
                Seconds(f64::NAN)
            ),
            Err(AcousticsError::NotFinite)
        ));
        assert!(matches!(
            solve_emission_time(
                |t| Ecef::new(t.0, f64::INFINITY, 0.0),
                observer,
                340.0,
                Seconds(1.0)
            ),
            Err(AcousticsError::NotFinite)
        ));
    }

    #[test]
    fn solver_reports_no_solution_when_the_bracket_runs_out() {
        // A source racing directly away faster than sound keeps the residual
        // positive for every earlier time, so no emission time exists.
        let c = sound_speed();
        let reception_time = Seconds(10.0);
        let result = solve_emission_time(
            |emission_time| Ecef::new(2.0 * c * (reception_time.0 - emission_time.0), 0.0, 0.0),
            Ecef::ORIGIN,
            c,
            reception_time,
        );
        assert!(matches!(result, Err(AcousticsError::NoRealSolution)));
    }
}
