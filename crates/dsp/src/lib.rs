//! Resampling, windowing and short-time spectral analysis.
//!
//! `docs/SDD.md` section 5 fixes the front end: decode to mono, band-limit to
//! roughly 20-1500 Hz, decimate with a Kaiser FIR, remove DC, then form a
//! whitened STFT. This crate starts with the window and frame geometry; the
//! transform and whitening stages are added with their tests.

pub mod error;
pub mod stft;
pub mod window;

pub use error::DspError;
pub use stft::StftConfig;
pub use window::{WindowKind, window};
