//! The two clocks used throughout the project (SDD section 4).
//!
//! `AudioSeconds` is the audio clock `t_a`, measured from the start of the
//! file. `UtcMillis` is UTC `t_u`. The unknown recording start `s` relates the
//! two: `t_u = s + t_a`. Keeping them as different types prevents a conversion
//! from happening by accident.
//!
//! The conversion functions are the only place where the two clocks meet
//! (Norms section 5). Everything else carries one clock or the other.

use core::fmt;
use core::ops::{Add, AddAssign, Sub};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Audio-clock time in seconds from the start of the recording.
///
/// Negative values are allowed because a detection can be described relative
/// to a fitted centre rather than the file start.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct AudioSeconds(pub f64);

/// UTC time in milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

impl AudioSeconds {
    /// Seconds since the audio start, as a bare `f64`.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Return `true` when the value is finite.
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl Add for AudioSeconds {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for AudioSeconds {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for AudioSeconds {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl fmt::Display for AudioSeconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3} s (audio)", self.0)
    }
}

impl UtcMillis {
    /// Current wall-clock time as milliseconds since the Unix epoch.
    ///
    /// Saturates at the representable range if the system clock is before the
    /// epoch or beyond `i64` milliseconds.
    #[must_use]
    pub fn now() -> Self {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        Self(i64::try_from(millis).unwrap_or(i64::MAX))
    }

    /// Milliseconds since the Unix epoch as a bare `i64`.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The seconds part, truncated toward zero.
    ///
    /// Used to split a UTC instant into whole seconds plus a sub-second
    /// remainder when formatting ISO-8601 strings.
    pub const fn whole_seconds(self) -> i64 {
        self.0.div_euclid(1_000)
    }

    /// The sub-second remainder in the range `0..1000`.
    pub const fn sub_millis(self) -> i64 {
        self.0.rem_euclid(1_000)
    }

    /// Difference `self - other` as a signed number of milliseconds.
    pub const fn difference(self, other: Self) -> i64 {
        self.0 - other.0
    }
}

impl Add<AudioSeconds> for UtcMillis {
    type Output = Self;
    /// Shift a UTC instant by an audio-clock offset.
    ///
    /// This is `t_u + t_a`; the caller is responsible for applying the sign
    /// that is right for the recording start.
    fn add(self, rhs: AudioSeconds) -> Self::Output {
        Self(self.0 + rhs.0.mul_add(1_000.0, 0.0).round() as i64)
    }
}

impl Sub for UtcMillis {
    type Output = AudioSeconds;
    /// Difference of two UTC instants as an audio-clock-compatible interval.
    fn sub(self, rhs: Self) -> Self::Output {
        AudioSeconds((self.0 - rhs.0) as f64 / 1_000.0)
    }
}

impl fmt::Display for UtcMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ms (UTC)", self.0)
    }
}

/// The unknown recording start `s` (SDD section 4, section 6.2).
///
/// This is the bridge between the two clocks. `s` is a UTC instant: the
/// absolute time at which audio time zero occurs. Because `s` is never known
/// exactly, the value carries its uncertainty as a scalar variance in
/// seconds squared. The search fuses the per-event implied starts through
/// this variance rather than through a hard offset filter (SDD section 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecordingStart {
    /// Estimated UTC instant of audio time zero.
    pub utc: UtcMillis,
    /// Variance of the estimate, in seconds squared. Always finite and > 0.
    pub variance_s2: f64,
}

impl RecordingStart {
    /// Build a bridge from a UTC instant and its variance in seconds squared.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::NotFinite`] when the variance is NaN or infinite,
    /// and [`CoreError::NonPositiveVariance`] when it is zero or negative.
    pub fn new(utc: UtcMillis, variance_s2: f64) -> Result<Self, CoreError> {
        if !variance_s2.is_finite() {
            return Err(CoreError::NotFinite);
        }
        if variance_s2 <= 0.0 {
            return Err(CoreError::NonPositiveVariance(variance_s2));
        }
        Ok(Self { utc, variance_s2 })
    }

    /// Build a bridge from a standard deviation in seconds.
    ///
    /// # Errors
    ///
    /// As [`RecordingStart::new`].
    pub fn from_std_dev(utc: UtcMillis, std_dev_s: f64) -> Result<Self, CoreError> {
        Self::new(utc, std_dev_s * std_dev_s)
    }

    /// Standard deviation of the estimate, in seconds.
    pub fn std_dev_s(self) -> f64 {
        self.variance_s2.sqrt()
    }

    /// Convert an audio instant to UTC through this bridge.
    ///
    /// This is `t_u = s + t_a`, rounded to the nearest millisecond. The
    /// returned UTC instant inherits the same variance as `s`, since audio
    /// time is exact.
    pub fn to_utc(self, t_audio: AudioSeconds) -> UtcMillis {
        utc_from_audio(t_audio, self.utc)
    }

    /// Convert a UTC instant to audio time through this bridge.
    ///
    /// This is `t_a = t_u - s`, in seconds.
    pub fn to_audio(self, t_utc: UtcMillis) -> AudioSeconds {
        audio_from_utc(t_utc, self.utc)
    }

    /// The start implied by pairing a heard audio instant with its UTC instant.
    ///
    /// `s = t_u - t_a`. The variance must be supplied by the caller because it
    /// depends on how well the UTC instant is known (fit plus ADS-B timing).
    ///
    /// # Errors
    ///
    /// As [`RecordingStart::new`].
    pub fn implied(
        t_audio: AudioSeconds,
        t_utc: UtcMillis,
        variance_s2: f64,
    ) -> Result<Self, CoreError> {
        let offset_millis = (t_audio.0 * 1_000.0).round() as i64;
        Self::new(UtcMillis(t_utc.0 - offset_millis), variance_s2)
    }
}

/// Convert audio time to UTC using the recording start `s`.
///
/// `t_u = s + t_a`, rounded to the nearest millisecond.
pub fn utc_from_audio(t_audio: AudioSeconds, recording_start: UtcMillis) -> UtcMillis {
    UtcMillis(recording_start.0 + (t_audio.0 * 1_000.0).round() as i64)
}

/// Convert UTC time to audio time using the recording start `s`.
///
/// `t_a = t_u - s`, in seconds.
pub fn audio_from_utc(t_utc: UtcMillis, recording_start: UtcMillis) -> AudioSeconds {
    AudioSeconds((t_utc.0 - recording_start.0) as f64 / 1_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_to_utc_adds_the_start() {
        let start = UtcMillis(1_700_000_000_000);
        let utc = utc_from_audio(AudioSeconds(12.5), start);
        assert_eq!(utc, UtcMillis(1_700_000_012_500));
    }

    #[test]
    fn utc_to_audio_subtracts_the_start() {
        let start = UtcMillis(1_700_000_000_000);
        let audio = audio_from_utc(UtcMillis(1_700_000_012_500), start);
        assert!((audio.0 - 12.5).abs() < 1e-9);
    }

    #[test]
    fn round_trip_is_exact_to_the_millisecond() {
        let start = UtcMillis(1_700_000_000_000);
        for millis in 0..2_000 {
            let audio = AudioSeconds(f64::from(millis) / 1_000.0);
            let back = audio_from_utc(utc_from_audio(audio, start), start);
            assert!((back.0 - audio.0).abs() < 1e-9, "{back:?} != {audio:?}");
        }
    }

    #[test]
    fn negative_audio_time_represents_before_file_start() {
        let start = UtcMillis(1_700_000_000_000);
        let utc = utc_from_audio(AudioSeconds(-0.25), start);
        assert_eq!(utc.0, 1_700_000_000_000 - 250);
    }

    #[test]
    fn subtraction_of_utc_instants_gives_a_seconds_interval() {
        let a = UtcMillis(1_700_000_010_000);
        let b = UtcMillis(1_700_000_000_000);
        assert!((a - b).0 - 10.0 < 1e-9);
    }

    #[test]
    fn whole_seconds_and_remainder_recombine() {
        let value = UtcMillis(1_700_000_012_345);
        assert_eq!(value.whole_seconds() * 1_000 + value.sub_millis(), value.0);
    }

    #[test]
    fn negative_instants_split_consistently() {
        let value = UtcMillis(-1_500);
        assert_eq!(value.whole_seconds(), -2);
        assert_eq!(value.sub_millis(), 500);
        assert_eq!(value.whole_seconds() * 1_000 + value.sub_millis(), -1_500);
    }

    #[test]
    fn recording_start_bridge_round_trips() {
        let bridge = RecordingStart::new(UtcMillis(1_700_000_000_000), 0.25).expect("valid");
        let t_audio = AudioSeconds(12.5);
        let t_utc = bridge.to_utc(t_audio);
        assert_eq!(t_utc, UtcMillis(1_700_000_012_500));
        let back = bridge.to_audio(t_utc);
        assert!((back.0 - t_audio.0).abs() < 1e-9);
    }

    #[test]
    fn recording_start_carries_variance() {
        let bridge = RecordingStart::from_std_dev(UtcMillis(0), 0.5).expect("valid");
        assert!((bridge.variance_s2 - 0.25).abs() < 1e-12);
        assert!((bridge.std_dev_s() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn recording_start_implied_recovers_s() {
        let start = UtcMillis(1_700_000_000_000);
        let t_audio = AudioSeconds(3.0);
        let t_utc = utc_from_audio(t_audio, start);
        let implied = RecordingStart::implied(t_audio, t_utc, 0.01).expect("valid");
        assert_eq!(implied.utc, start);
    }

    #[test]
    fn recording_start_rejects_bad_variance() {
        assert!(matches!(
            RecordingStart::new(UtcMillis(0), f64::NAN),
            Err(CoreError::NotFinite)
        ));
        assert!(matches!(
            RecordingStart::new(UtcMillis(0), 0.0),
            Err(CoreError::NonPositiveVariance(_))
        ));
        assert!(matches!(
            RecordingStart::new(UtcMillis(0), -1.0),
            Err(CoreError::NonPositiveVariance(_))
        ));
    }
}
