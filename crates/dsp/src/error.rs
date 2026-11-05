//! DSP configuration failures.

/// A failure in a DSP configuration or stage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DspError {
    /// A window or frame length was zero.
    #[error("length must be greater than zero")]
    ZeroLength,
    /// The hop length was zero or larger than the frame length.
    #[error("hop length {hop} is invalid for frame length {frame}")]
    InvalidHop {
        /// Requested hop length in samples.
        hop: usize,
        /// Requested frame length in samples.
        frame: usize,
    },
    /// A sample rate or other scalar was NaN or infinite.
    #[error("value must be finite")]
    NotFinite,
    /// The requested frame does not fit in the input.
    #[error("input of {samples} samples is shorter than frame length {frame}")]
    InputTooShort {
        /// Number of samples supplied.
        samples: usize,
        /// Required frame length in samples.
        frame: usize,
    },
    /// The input contained no samples.
    #[error("input must contain at least one sample")]
    EmptyInput,
    /// A sample rate was zero or negative.
    #[error("sample rates must be positive, got input {input_hz} Hz and output {output_hz} Hz")]
    InvalidRate {
        /// Requested input rate in hertz.
        input_hz: f64,
        /// Requested output rate in hertz.
        output_hz: f64,
    },
    /// The analysis band edges are not usable.
    #[error(
        "low cut {low_cut_hz} Hz, passband {passband_hz} Hz and stopband {stopband_hz} Hz \
         must satisfy 0 < low cut < passband < stopband < {nyquist_hz} Hz"
    )]
    InvalidBand {
        /// Requested lower band edge in hertz.
        low_cut_hz: f64,
        /// Requested passband edge in hertz.
        passband_hz: f64,
        /// Requested stopband edge in hertz.
        stopband_hz: f64,
        /// Nyquist frequency of the slower side in hertz.
        nyquist_hz: f64,
    },
    /// The requested stopband attenuation was not positive.
    #[error("stopband attenuation must be positive, got {0} dB")]
    InvalidAttenuation(f64),
    /// The designed anti-alias filter exceeded the implementation limit.
    #[error("designed filter has {taps} taps, above the limit of {max_taps}")]
    FilterTooLong {
        /// Number of taps the design requested.
        taps: usize,
        /// Largest supported tap count.
        max_taps: usize,
    },
    /// The anti-alias filter had no usable energy.
    #[error("anti-alias filter design produced no usable taps")]
    InvalidFilter,
    /// A length or index calculation overflowed.
    #[error("sample or filter length overflowed")]
    LengthOverflow,
    /// The sample rates could not be expressed as a bounded rational ratio.
    #[error("sample rates {input_hz} Hz and {output_hz} Hz do not form a supported ratio")]
    UnsupportedRate {
        /// Requested input rate in hertz.
        input_hz: f64,
        /// Requested output rate in hertz.
        output_hz: f64,
    },
}
