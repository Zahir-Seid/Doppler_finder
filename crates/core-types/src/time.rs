//! The two clocks used throughout the project (SDD section 4).
//!
//! `AudioSeconds` is the audio clock `t_a`, measured from the start of the
//! file. `UtcMillis` is UTC `t_u`. The unknown recording start `s` relates the
//! two: `t_u = s + t_a`. Keeping them as different types prevents a conversion
//! from happening by accident.

use serde::{Deserialize, Serialize};

/// Audio-clock time in seconds from the start of the recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudioSeconds(pub f64);

/// UTC time in milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

/// Convert audio time to UTC using the recording start `s`.
///
/// This is the only place where the two clocks are combined (Norms section 5).
pub fn utc_from_audio(t_audio: AudioSeconds, recording_start: UtcMillis) -> UtcMillis {
    UtcMillis(recording_start.0 + (t_audio.0 * 1_000.0).round() as i64)
}

/// Convert UTC time to audio time using the recording start `s`.
pub fn audio_from_utc(t_utc: UtcMillis, recording_start: UtcMillis) -> AudioSeconds {
    AudioSeconds((t_utc.0 - recording_start.0) as f64 / 1_000.0)
}
