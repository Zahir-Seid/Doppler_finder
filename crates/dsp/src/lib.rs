//! Resampling, windowing and short-time spectral analysis.
//!
//! `docs/SDD.md` section 5 fixes the front end: decode to mono, band-limit to
//! roughly 20-1500 Hz, decimate with a Kaiser FIR, remove DC, then form a
//! whitened STFT. The band-limit and rate change live in [`frontend`]; the
//! windowed transform and its sub-bin interpolation live in [`stft`]. Whitening
//! and ridge tracking are added with their own tasks (`DSP-04`, `DSP-05`).

pub mod error;
pub mod frontend;
pub mod stft;
pub mod window;

pub use error::DspError;
pub use frontend::{
    DEFAULT_ATTENUATION_DB, DEFAULT_LOW_CUT_HZ, DEFAULT_OUTPUT_RATE_HZ, DEFAULT_PASSBAND_HZ,
    DEFAULT_STOPBAND_HZ, FrontEndConfig, remove_dc, resample_to_analysis_rate,
};
pub use stft::{DEFAULT_OVERLAP_FRACTION, SURVEY_WINDOW_S, Spectrogram, StftConfig, spectrogram};
pub use window::{DEFAULT_KAISER_BETA, WindowKind, window};
