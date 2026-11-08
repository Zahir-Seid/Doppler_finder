//! Measured track sampling statistics (FR-DA-01).
//!
//! FR-DA-01 records the source's sampling interval in the run manifest and
//! SDD section 9.3 stores median and 90th-percentile gaps per source and day.
//! [`sampling_stats`] computes both from the intervals actually present in a
//! batch, so the error budget uses measurements rather than assumptions.

use core_types::DataSource;

use crate::tracks::FlightTrack;

/// Sampling statistics for one archive batch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingStats {
    /// Number of track samples represented.
    pub sample_count: usize,
    /// Number of positive intervals between consecutive samples.
    pub interval_count: usize,
    /// Median interval in seconds.
    pub median_s: f64,
    /// 90th-percentile interval in seconds, nearest-rank.
    pub p90_s: f64,
}

impl SamplingStats {
    /// Build the manifest [`DataSource`] record for this batch.
    ///
    /// `source` is the provider name, `version` the source's own version or
    /// date label, and `file_sha256` the hash of the consumed payload.
    pub fn data_source(
        &self,
        source: impl Into<String>,
        version: impl Into<String>,
        file_sha256: impl Into<String>,
    ) -> DataSource {
        DataSource {
            source: source.into(),
            version: version.into(),
            file_sha256: file_sha256.into(),
            sampling_median_s: self.median_s,
            sampling_p90_s: self.p90_s,
        }
    }
}

/// Compute sampling statistics from all positive consecutive gaps.
///
/// Duplicate timestamps are ignored because they do not describe the source's
/// sampling interval. Returns `None` when no positive gap exists.
pub fn sampling_stats(tracks: &[FlightTrack]) -> Option<SamplingStats> {
    let sample_count = tracks
        .iter()
        .map(|track| track.samples.len())
        .sum::<usize>();
    let mut intervals = Vec::new();
    for track in tracks {
        for pair in track.samples.windows(2) {
            let (Some(first), Some(second)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let seconds = (second.utc.0 - first.utc.0) as f64 / 1000.0;
            if seconds.is_finite() && seconds > 0.0 {
                intervals.push(seconds);
            }
        }
    }
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_by(f64::total_cmp);
    let count = intervals.len();
    let median_s = if count % 2 == 1 {
        *intervals.get(count / 2)?
    } else {
        let lower = *intervals.get(count / 2 - 1)?;
        let upper = *intervals.get(count / 2)?;
        f64::midpoint(lower, upper)
    };
    let rank = ((count as f64) * 0.9).ceil() as usize;
    let p90_index = rank.saturating_sub(1).min(count - 1);
    let p90_s = *intervals.get(p90_index)?;

    Some(SamplingStats {
        sample_count,
        interval_count: count,
        median_s,
        p90_s,
    })
}
