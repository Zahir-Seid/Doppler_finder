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
    /// The transform length was shorter than the frame or not even.
    #[error("FFT length {fft_len} is invalid for frame length {frame}")]
    InvalidFftLength {
        /// Requested transform length in samples.
        fft_len: usize,
        /// Frame length in samples.
        frame: usize,
    },
    /// A window shape parameter was NaN, infinite or negative.
    #[error("window shape parameter {beta} must be finite and non-negative")]
    InvalidWindow {
        /// Requested Kaiser shape parameter.
        beta: f64,
    },
    /// The real FFT rejected its input or output buffers.
    #[error("the real FFT rejected its input or output buffers")]
    TransformFailed,
    /// A mask or map did not carry one value per frame and bin.
    #[error("expected {expected} values, got {actual}")]
    ShapeMismatch {
        /// Number of values the geometry requires.
        expected: usize,
        /// Number of values supplied.
        actual: usize,
    },
    /// A frequency corridor did not cover exactly the map's frames.
    #[error("corridor covers {actual} frames but the map has {expected}")]
    CorridorMismatch {
        /// Number of frames the map has.
        expected: usize,
        /// Number of frames the corridor covers.
        actual: usize,
    },
    /// A corridor bound was reversed or past the last bin.
    #[error("corridor bounds {low}..={high} are invalid for {bins} bins")]
    InvalidCorridor {
        /// Lower bin bound.
        low: usize,
        /// Upper bin bound.
        high: usize,
        /// Number of bins in the map.
        bins: usize,
    },
    /// A variance, cost or similar scalar was negative or not finite.
    #[error("value {0} must be finite and non-negative")]
    InvalidVariance(f64),
    /// A frequency range was crossed or not finite.
    #[error("frequency range {low_hz}..{high_hz} Hz is not usable")]
    InvalidFrequencyRange {
        /// Lower edge in hertz.
        low_hz: f64,
        /// Upper edge in hertz.
        high_hz: f64,
    },
    /// The media file is missing or not a regular file.
    #[error("media file is missing or is not a regular file")]
    MediaNotFound,
    /// The media file is larger than the configured cap.
    #[error("media file is {bytes} bytes, above the limit of {limit}")]
    MediaTooLarge {
        /// Actual size in bytes.
        bytes: u64,
        /// Configured limit in bytes.
        limit: u64,
    },
    /// The decoder program could not be started.
    #[error("the media decoder could not be started")]
    DecoderUnavailable,
    /// The decoder exceeded its wall-clock limit.
    #[error("the media decoder exceeded its {limit_s} s limit")]
    DecoderTimeout {
        /// Configured limit in seconds.
        limit_s: f64,
    },
    /// The decoder exited unsuccessfully.
    #[error("the media decoder exited with status {code:?}")]
    DecoderFailed {
        /// Process exit code, when the process exited normally.
        code: Option<i32>,
    },
    /// The decoder returned more audio than the duration limit allows.
    #[error("the media decoder returned more than {limit_bytes} bytes")]
    DecoderOutputTooLarge {
        /// Configured output limit in bytes.
        limit_bytes: usize,
    },
    /// The decoded stream is not audio the analyser can use.
    #[error("decoded media is unusable: {problem:?}")]
    InvalidMediaOutput {
        /// What was wrong with the stream.
        problem: MediaProblem,
    },
    /// The decoded clip is longer than the configured limit.
    #[error("decoded clip is {duration_s} s, above the limit of {limit_s} s")]
    AudioTooLong {
        /// Decoded duration in seconds.
        duration_s: f64,
        /// Configured limit in seconds.
        limit_s: f64,
    },
}

/// What the decoder returned that the analyser cannot use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaProblem {
    /// The byte stream was not a usable WAV.
    Header,
    /// The sample format was not 32-bit float.
    SampleFormat,
    /// The stream was not mono.
    Channels,
    /// The sample rate was not the requested rate.
    SampleRate,
    /// The data chunk was not a whole number of samples.
    DataLength,
}
