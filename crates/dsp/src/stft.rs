//! Short-time Fourier transform geometry (SDD section 5).

use crate::error::DspError;

/// Frame geometry for a short-time Fourier transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StftConfig {
    /// Sample rate of the (possibly decimated) input in hertz.
    pub sample_rate_hz: f64,
    /// Samples per frame; the FFT length is the next power of two.
    pub frame_len: usize,
    /// Samples between successive frame starts.
    pub hop_len: usize,
}

impl StftConfig {
    /// Validate the geometry.
    ///
    /// Returns [`DspError`] for a zero length, a hop outside `1..=frame_len`, or
    /// a non-finite sample rate.
    pub fn validate(&self) -> Result<(), DspError> {
        if self.frame_len == 0 {
            return Err(DspError::ZeroLength);
        }
        if self.hop_len == 0 || self.hop_len > self.frame_len {
            return Err(DspError::InvalidHop {
                hop: self.hop_len,
                frame: self.frame_len,
            });
        }
        if !self.sample_rate_hz.is_finite() || self.sample_rate_hz <= 0.0 {
            return Err(DspError::NotFinite);
        }
        Ok(())
    }

    /// Number of complete frames in an input of `sample_count` samples.
    ///
    /// Returns [`DspError::InputTooShort`] when the input cannot hold one
    /// complete frame.
    pub fn frame_count(&self, sample_count: usize) -> Result<usize, DspError> {
        self.validate()?;
        if sample_count < self.frame_len {
            return Err(DspError::InputTooShort {
                samples: sample_count,
                frame: self.frame_len,
            });
        }
        Ok((sample_count - self.frame_len) / self.hop_len + 1)
    }
}
