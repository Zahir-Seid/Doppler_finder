//! Media decoding seam and the FFmpeg subprocess adapter (FR-IN-01, SDD T-1).
//!
//! Decoding untrusted media runs in a child process, never in a linked library:
//! FFmpeg's licence stays out of the binary and a malformed file can only kill
//! the child. The parent treats the child's output as untrusted and checks the
//! WAV it returns for sample format, channel count, rate, length, duration and
//! finiteness before any stage sees a sample.
//!
//! The requested rate is imposed with `-ar` and then re-read from the WAV
//! header, so a decoder that ignored the request is rejected rather than
//! silently analysed at the wrong rate.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{DspError, MediaProblem};

/// Default wall-clock limit for one decode.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default largest accepted media file, in bytes.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Default longest accepted clip, in seconds.
///
/// Matches the local research budget on the CLI (SDD 11.3); the service applies
/// its own shorter cap before it reaches this decoder.
pub const DEFAULT_MAX_DURATION_S: f64 = 600.0;

/// WAV format tag for IEEE floating-point samples.
const WAV_FORMAT_FLOAT: u16 = 3;

/// Bytes of header slack allowed above the sample-data limit.
const WAV_HEADER_SLACK_BYTES: usize = 1 << 20;

/// Largest stderr capture kept from the decoder, in bytes.
const STDERR_CAPTURE_BYTES: usize = 8 * 1024;

/// Read buffer size for the decoder pipes, in bytes.
const PIPE_CHUNK_BYTES: usize = 8 * 1024;

/// Settings for the media decoder seam.
#[derive(Debug, Clone, PartialEq)]
pub struct DecoderConfig {
    /// Rate the decoder must produce, in hertz.
    pub sample_rate_hz: f64,
    /// Largest accepted input file, in bytes.
    pub max_file_bytes: u64,
    /// Longest accepted output duration, in seconds.
    pub max_duration_s: f64,
    /// Wall-clock limit for the decoder process.
    pub timeout: Duration,
}

impl DecoderConfig {
    /// Build the default configuration for a sample rate.
    pub fn for_sample_rate(sample_rate_hz: f64) -> Self {
        Self {
            sample_rate_hz,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_duration_s: DEFAULT_MAX_DURATION_S,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Validate the settings before any process runs.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::InvalidRate`] for a non-positive rate,
    /// [`DspError::AudioTooLong`] for a non-positive duration limit, and
    /// [`DspError::ZeroLength`] for a zero file cap or timeout.
    pub fn validate(&self) -> Result<(), DspError> {
        if !self.sample_rate_hz.is_finite() || self.sample_rate_hz <= 0.0 {
            return Err(DspError::InvalidRate {
                input_hz: self.sample_rate_hz,
                output_hz: self.sample_rate_hz,
            });
        }
        if !self.max_duration_s.is_finite() || self.max_duration_s <= 0.0 {
            return Err(DspError::AudioTooLong {
                duration_s: self.max_duration_s,
                limit_s: self.max_duration_s,
            });
        }
        if self.max_file_bytes == 0 || self.timeout.is_zero() {
            return Err(DspError::ZeroLength);
        }
        Ok(())
    }

    /// Largest number of decoded bytes accepted for this configuration.
    fn decoded_limit_bytes(&self) -> usize {
        let samples = self.max_duration_s * self.sample_rate_hz;
        let bytes = samples.max(0.0).min(usize::MAX as f64) as usize;
        bytes
            .saturating_mul(4)
            .saturating_add(WAV_HEADER_SLACK_BYTES)
    }
}

/// Mono PCM decoded from a media file.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    samples: Vec<f32>,
    sample_rate_hz: f64,
    decoder_version: Option<String>,
}

impl DecodedAudio {
    /// Build decoded audio, validating the rate and every sample.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::InvalidRate`] for a non-positive or non-finite rate,
    /// and [`DspError::NotFinite`] if any sample is NaN or infinite.
    pub fn new(
        samples: Vec<f32>,
        sample_rate_hz: f64,
        decoder_version: Option<String>,
    ) -> Result<Self, DspError> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(DspError::InvalidRate {
                input_hz: sample_rate_hz,
                output_hz: sample_rate_hz,
            });
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(DspError::NotFinite);
        }
        Ok(Self {
            samples,
            sample_rate_hz,
            decoder_version,
        })
    }

    /// Mono samples.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Sample rate of the decoded audio, in hertz.
    pub fn sample_rate_hz(&self) -> f64 {
        self.sample_rate_hz
    }

    /// Duration of the decoded audio, in seconds.
    pub fn duration_s(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate_hz
    }

    /// First line of the decoder's version output, when it reported one.
    ///
    /// This belongs in the run manifest so a result can be reproduced with the
    /// same decoder build (NFR-04).
    pub fn decoder_version(&self) -> Option<&str> {
        self.decoder_version.as_deref()
    }

    /// Take the samples, consuming the record.
    pub fn into_samples(self) -> Vec<f32> {
        self.samples
    }
}

/// The seam every media decoder implements.
///
/// Tests and fixtures can supply an in-memory implementation, so nothing below
/// this trait depends on a decoder being installed.
pub trait MediaDecoder {
    /// Decode a media file to mono PCM at the configured rate.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the file is missing or too large, the decoder
    /// cannot run or times out, or its output is unusable.
    fn decode(&self, path: &Path) -> Result<DecodedAudio, DspError>;
}

/// FFmpeg-based decoder that runs the codec tool as a child process.
#[derive(Debug, Clone)]
pub struct FfmpegDecoder {
    config: DecoderConfig,
    program: PathBuf,
}

impl FfmpegDecoder {
    /// Build a decoder that runs `ffmpeg` from the search path.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the configuration is invalid.
    pub fn new(config: DecoderConfig) -> Result<Self, DspError> {
        Self::with_program(config, "ffmpeg")
    }

    /// Build a decoder that runs a specific program, for tests and packaging.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the configuration is invalid.
    pub fn with_program(
        config: DecoderConfig,
        program: impl Into<PathBuf>,
    ) -> Result<Self, DspError> {
        config.validate()?;
        Ok(Self {
            config,
            program: program.into(),
        })
    }

    /// The configuration in use.
    pub fn config(&self) -> &DecoderConfig {
        &self.config
    }

    /// First line of `ffmpeg -version`.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::DecoderUnavailable`] when the program cannot be
    /// started and [`DspError::DecoderFailed`] when it exits unsuccessfully.
    pub fn version(&self) -> Result<String, DspError> {
        let output = Command::new(&self.program)
            .arg("-version")
            .stdin(Stdio::null())
            .output()
            .map_err(|_error| DspError::DecoderUnavailable)?;
        if !output.status.success() {
            return Err(DspError::DecoderFailed {
                code: output.status.code(),
            });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().next().unwrap_or_default().trim().to_string())
    }

    /// Arguments the child is run with: mono 32-bit float WAV on stdout.
    fn arguments(&self, path: &Path) -> Vec<std::ffi::OsString> {
        let rate = self.config.sample_rate_hz.round().max(1.0).to_string();
        vec![
            "-v".into(),
            "error".into(),
            "-nostdin".into(),
            "-i".into(),
            path.as_os_str().to_owned(),
            "-map".into(),
            "0:a:0".into(),
            "-ac".into(),
            "1".into(),
            "-ar".into(),
            rate.into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            "-f".into(),
            "wav".into(),
            "pipe:1".into(),
        ]
    }

    /// Start the decoder child process.
    fn spawn(&self, path: &Path) -> Result<Child, DspError> {
        Command::new(&self.program)
            .args(self.arguments(path))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_error| DspError::DecoderUnavailable)
    }
}

impl MediaDecoder for FfmpegDecoder {
    fn decode(&self, path: &Path) -> Result<DecodedAudio, DspError> {
        self.config.validate()?;
        check_file(&self.config, path)?;
        let mut child = self.spawn(path)?;
        let Some(stdout) = child.stdout.take() else {
            return Err(DspError::DecoderUnavailable);
        };
        let Some(stderr) = child.stderr.take() else {
            return Err(DspError::DecoderUnavailable);
        };
        let stderr_drain = drain_stderr(stderr);
        let limit = self.config.decoded_limit_bytes();
        let (bytes, overflowed) = read_stdout(&mut child, stdout, limit, self.config.timeout)?;
        let status = child
            .wait()
            .map_err(|_error| DspError::DecoderFailed { code: None })?;
        // The capture is diagnostic only, so a panicked drain is not fatal.
        let _ = stderr_drain.join();

        if overflowed {
            return Err(DspError::DecoderOutputTooLarge { limit_bytes: limit });
        }
        if !status.success() {
            return Err(DspError::DecoderFailed {
                code: status.code(),
            });
        }
        let (samples, sample_rate_hz) = validate_output(&self.config, &bytes)?;
        // A decoder that works but prints no version still yields audio; the
        // manifest then records that no version was available.
        let decoder_version = self.version().ok().filter(|line| !line.is_empty());
        DecodedAudio::new(samples, sample_rate_hz, decoder_version)
    }
}

/// Reject a missing file, or one above the configured size cap, before running
/// the decoder.
fn check_file(config: &DecoderConfig, path: &Path) -> Result<(), DspError> {
    if !path.is_file() {
        return Err(DspError::MediaNotFound);
    }
    let bytes = std::fs::metadata(path)
        .map_err(|_error| DspError::MediaNotFound)?
        .len();
    if bytes > config.max_file_bytes {
        return Err(DspError::MediaTooLarge {
            bytes,
            limit: config.max_file_bytes,
        });
    }
    Ok(())
}

/// Drain the decoder's stderr so a chatty child cannot block on a full pipe.
///
/// The text is diagnostic only and never leaves this function: it may quote the
/// media path, which must not reach a log or an error message.
fn drain_stderr(stderr: ChildStderr) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let mut reader = BufReader::new(stderr);
        let mut chunk = [0_u8; PIPE_CHUNK_BYTES];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if sink.len() < STDERR_CAPTURE_BYTES
                        && let Some(piece) = chunk.get(..count)
                    {
                        sink.extend_from_slice(piece);
                    }
                }
            }
        }
    })
}

/// Read the child's stdout up to `limit` bytes, killing it on timeout.
///
/// Returns the bytes and whether the limit was hit.
fn read_stdout(
    child: &mut Child,
    stdout: ChildStdout,
    limit: usize,
    timeout: Duration,
) -> Result<(Vec<u8>, bool), DspError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut overflowed = false;
        let mut reader = BufReader::new(stdout);
        let mut chunk = [0_u8; PIPE_CHUNK_BYTES];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if buffer.len().saturating_add(count) > limit {
                        overflowed = true;
                        break;
                    }
                    if let Some(piece) = chunk.get(..count) {
                        buffer.extend_from_slice(piece);
                    }
                }
            }
        }
        // The receiver is gone when the parent has already timed out.
        let _ = sender.send((buffer, overflowed));
    });

    let Ok(received) = receiver.recv_timeout(timeout) else {
        // The child is killed and reaped; nothing reads its output now.
        let _ = child.kill();
        let _ = child.wait();
        return Err(DspError::DecoderTimeout {
            limit_s: timeout.as_secs_f64(),
        });
    };
    Ok(received)
}

/// Check the WAV the decoder returned and turn it into samples.
fn validate_output(config: &DecoderConfig, bytes: &[u8]) -> Result<(Vec<f32>, f64), DspError> {
    let (sample_rate_hz, pcm) = parse_wav(bytes)?;
    if (sample_rate_hz - config.sample_rate_hz).abs() > 1.0e-6 {
        return Err(DspError::InvalidMediaOutput {
            problem: MediaProblem::SampleRate,
        });
    }
    let samples = decode_float_samples(&pcm)?;
    if samples.is_empty() {
        return Err(DspError::EmptyInput);
    }
    let duration_s = samples.len() as f64 / sample_rate_hz;
    if duration_s > config.max_duration_s {
        return Err(DspError::AudioTooLong {
            duration_s,
            limit_s: config.max_duration_s,
        });
    }
    Ok((samples, sample_rate_hz))
}

/// Read the rate and sample data out of a mono float WAV stream.
///
/// Only the parts the analyser needs are read: the format chunk is validated
/// (32-bit float, mono) and the data chunk is copied out.
fn parse_wav(bytes: &[u8]) -> Result<(f64, Vec<u8>), DspError> {
    let header_error = DspError::InvalidMediaOutput {
        problem: MediaProblem::Header,
    };
    if bytes.get(0..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"WAVE".as_slice()) {
        return Err(header_error);
    }
    let mut cursor = 12_usize;
    let mut format = None;
    let mut data = None;
    while let Some(header) = bytes.get(cursor..cursor.saturating_add(8)) {
        let Some(id) = header.get(0..4) else {
            break;
        };
        let Some(size_field) = header.get(4..8) else {
            break;
        };
        let Ok(size_field) = <[u8; 4]>::try_from(size_field) else {
            break;
        };
        let size = u32::from_le_bytes(size_field) as usize;
        let payload_start = cursor.saturating_add(8);
        let payload_end = payload_start.saturating_add(size);
        if id == b"fmt ".as_slice() {
            format = bytes
                .get(payload_start..payload_end)
                .and_then(parse_fmt_chunk);
        } else if id == b"data".as_slice() {
            let end = payload_end.min(bytes.len());
            data = bytes.get(payload_start..end).map(<[u8]>::to_vec);
        }
        // Chunks are word-aligned.
        cursor = payload_start.saturating_add(size.saturating_add(size % 2));
    }
    let Some((format_tag, channels, sample_rate_hz, bits)) = format else {
        return Err(header_error);
    };
    let Some(data) = data else {
        return Err(header_error);
    };
    if format_tag != WAV_FORMAT_FLOAT || bits != 32 {
        return Err(DspError::InvalidMediaOutput {
            problem: MediaProblem::SampleFormat,
        });
    }
    if channels != 1 {
        return Err(DspError::InvalidMediaOutput {
            problem: MediaProblem::Channels,
        });
    }
    Ok((sample_rate_hz, data))
}

/// Read the fields of a `fmt ` chunk.
fn parse_fmt_chunk(payload: &[u8]) -> Option<(u16, u16, f64, u16)> {
    let read_u16 = |range: std::ops::Range<usize>| -> Option<u16> {
        let field = <[u8; 2]>::try_from(payload.get(range)?).ok()?;
        Some(u16::from_le_bytes(field))
    };
    let read_u32 = |range: std::ops::Range<usize>| -> Option<u32> {
        let field = <[u8; 4]>::try_from(payload.get(range)?).ok()?;
        Some(u32::from_le_bytes(field))
    };
    Some((
        read_u16(0..2)?,
        read_u16(2..4)?,
        f64::from(read_u32(4..8)?),
        read_u16(14..16)?,
    ))
}

/// Interpret little-endian 32-bit floats, rejecting a partial sample and any
/// non-finite value.
fn decode_float_samples(pcm: &[u8]) -> Result<Vec<f32>, DspError> {
    let (chunks, remainder) = pcm.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(DspError::InvalidMediaOutput {
            problem: MediaProblem::DataLength,
        });
    }
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let sample = f32::from_le_bytes(*chunk);
        if !sample.is_finite() {
            return Err(DspError::NotFinite);
        }
        samples.push(sample);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::{
        DecodedAudio, DecoderConfig, FfmpegDecoder, MediaDecoder, decode_float_samples, parse_wav,
    };
    use crate::error::{DspError, MediaProblem};

    /// A 16-bit mono PCM WAV with a tone at `frequency_hz`, written by hand so
    /// the test needs no encoder.
    fn write_wav(path: &Path, rate: u32, samples: &[i16]) -> std::io::Result<()> {
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes)
    }

    /// A mono tone of `duration_s` at `frequency_hz`.
    fn tone_samples(rate: u32, duration_s: f64, frequency_hz: f64) -> Vec<i16> {
        let count = (f64::from(rate) * duration_s).round() as usize;
        (0..count)
            .map(|index| {
                let time_s = index as f64 / f64::from(rate);
                let value = 0.5 * (std::f64::consts::TAU * frequency_hz * time_s).sin();
                (value * 32_767.0) as i16
            })
            .collect()
    }

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn decoder(config: DecoderConfig) -> FfmpegDecoder {
        FfmpegDecoder::new(config).expect("decoder")
    }

    #[test]
    fn fr_in_01_ffmpeg_decodes_a_wav_to_the_requested_rate() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("tone.wav");
        let samples = tone_samples(48_000, 0.5, 400.0);
        write_wav(&path, 48_000, &samples).expect("write fixture");

        let decoder = decoder(DecoderConfig::for_sample_rate(4_000.0));
        let decoded = decoder.decode(&path).expect("decode");
        assert!((decoded.sample_rate_hz() - 4_000.0).abs() < 1.0e-6);
        assert!(
            decoded.samples().len().abs_diff(2_000) <= 2,
            "decoded {} samples",
            decoded.samples().len()
        );
        assert!((decoded.duration_s() - 0.5).abs() < 0.01);
        assert!(decoded.samples().iter().all(|sample| sample.is_finite()));
        // A 0.5 amplitude sine has an RMS of 0.354.
        let rms = (decoded
            .samples()
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>()
            / decoded.samples().len() as f64)
            .sqrt();
        assert!((0.30..0.39).contains(&rms), "rms {rms}");
        assert!(decoded.decoder_version().is_some());
    }

    #[test]
    fn fr_in_01_a_clip_longer_than_the_limit_is_rejected() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("long.wav");
        write_wav(&path, 48_000, &tone_samples(48_000, 0.5, 400.0)).expect("write fixture");
        let config = DecoderConfig {
            max_duration_s: 0.05,
            ..DecoderConfig::for_sample_rate(4_000.0)
        };
        let result = decoder(config).decode(&path);
        assert!(matches!(result, Err(DspError::AudioTooLong { .. })));
    }

    #[test]
    fn fr_in_01_unsupported_media_is_rejected() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("notes.txt");
        std::fs::write(&path, b"this is not audio").expect("write fixture");
        let result = decoder(DecoderConfig::for_sample_rate(4_000.0)).decode(&path);
        assert!(matches!(
            result,
            Err(DspError::DecoderFailed { .. } | DspError::InvalidMediaOutput { .. })
        ));
    }

    #[test]
    fn fr_in_01_an_oversize_file_is_rejected_before_decoding() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("big.bin");
        std::fs::write(&path, vec![0_u8; 2_048]).expect("write fixture");
        let config = DecoderConfig {
            max_file_bytes: 1_024,
            ..DecoderConfig::for_sample_rate(4_000.0)
        };
        let result = decoder(config).decode(&path);
        assert!(matches!(result, Err(DspError::MediaTooLarge { .. })));
    }

    #[test]
    fn fr_in_01_a_missing_file_is_rejected() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("absent.wav");
        let result = decoder(DecoderConfig::for_sample_rate(4_000.0)).decode(&path);
        assert!(matches!(result, Err(DspError::MediaNotFound)));
    }

    #[test]
    fn fr_in_01_an_absent_decoder_program_is_reported() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("tone.wav");
        write_wav(&path, 48_000, &tone_samples(48_000, 0.01, 400.0)).expect("write fixture");
        let missing = FfmpegDecoder::with_program(
            DecoderConfig::for_sample_rate(4_000.0),
            "/nonexistent/ffmpeg-decoder",
        )
        .expect("decoder");
        let result = missing.decode(&path);
        assert!(matches!(result, Err(DspError::DecoderUnavailable)));
    }

    #[test]
    fn a_non_wav_stream_is_rejected() {
        assert!(matches!(
            parse_wav(b"nope"),
            Err(DspError::InvalidMediaOutput {
                problem: MediaProblem::Header
            })
        ));
        assert!(matches!(
            parse_wav(b""),
            Err(DspError::InvalidMediaOutput {
                problem: MediaProblem::Header
            })
        ));
    }

    #[test]
    fn a_stereo_stream_is_rejected() {
        let mut stream = build_float_wav(2, 4_000, &[0.0_f32; 8]);
        stream[22] = 2; // channel count in the fmt chunk
        assert!(matches!(
            parse_wav(&stream),
            Err(DspError::InvalidMediaOutput {
                problem: MediaProblem::Channels
            })
        ));
    }

    #[test]
    fn a_float_stream_yields_its_samples() {
        let samples = [0.25_f32, -0.5, 0.75];
        let stream = build_float_wav(1, 4_000, &samples);
        let (rate, pcm) = parse_wav(&stream).expect("wav");
        assert!((rate - 4_000.0).abs() < 1.0e-9);
        let decoded = decode_float_samples(&pcm).expect("samples");
        assert_eq!(decoded, samples);
    }

    #[test]
    fn a_partial_sample_or_non_finite_value_is_rejected() {
        assert!(matches!(
            decode_float_samples(&[0, 0, 0]),
            Err(DspError::InvalidMediaOutput {
                problem: MediaProblem::DataLength
            })
        ));
        let nan = f32::NAN.to_le_bytes();
        assert!(matches!(
            decode_float_samples(&nan),
            Err(DspError::NotFinite)
        ));
    }

    #[test]
    fn decoded_audio_validates_its_inputs() {
        assert!(matches!(
            DecodedAudio::new(vec![f32::NAN], 4_000.0, None),
            Err(DspError::NotFinite)
        ));
        assert!(matches!(
            DecodedAudio::new(vec![0.0], 0.0, None),
            Err(DspError::InvalidRate { .. })
        ));
        let audio = DecodedAudio::new(vec![0.0; 4_000], 4_000.0, Some("ffmpeg test".into()))
            .expect("audio");
        assert!((audio.duration_s() - 1.0).abs() < 1.0e-12);
        assert_eq!(audio.decoder_version(), Some("ffmpeg test"));
    }

    #[test]
    fn decoder_config_limits_are_validated() {
        let config = DecoderConfig {
            max_duration_s: 0.0,
            ..DecoderConfig::for_sample_rate(4_000.0)
        };
        assert!(config.validate().is_err());
        let config = DecoderConfig {
            max_file_bytes: 0,
            ..DecoderConfig::for_sample_rate(4_000.0)
        };
        assert!(config.validate().is_err());
        let config = DecoderConfig::for_sample_rate(-1.0);
        assert!(config.validate().is_err());
    }

    /// A mono or stereo 32-bit float WAV at the given rate.
    fn build_float_wav(channels: u16, rate: u32, samples: &[f32]) -> Vec<u8> {
        let data_len = (samples.len() * 4) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 4 * u32::from(channels)).to_le_bytes());
        bytes.extend_from_slice(&(4 * channels).to_le_bytes());
        bytes.extend_from_slice(&32_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }
}
