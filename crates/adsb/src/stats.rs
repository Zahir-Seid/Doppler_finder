//! Measured track sampling statistics (FR-DA-01).
//!
//! FR-DA-01 records the source's sampling interval and latency in the run
//! manifest and SDD section 9.3 stores median and 90th-percentile gaps per
//! source and day. [`sampling_stats`] computes all of them from the values
//! actually present in a batch, so the error budget uses measurements rather
//! than assumptions.

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
    /// Median reported position staleness in seconds, when reported.
    pub latency_median_s: Option<f64>,
    /// 90th-percentile reported position staleness in seconds.
    pub latency_p90_s: Option<f64>,
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
            latency_median_s: self.latency_median_s,
            latency_p90_s: self.latency_p90_s,
        }
    }
}

/// Compute sampling statistics from all positive consecutive gaps.
///
/// Duplicate timestamps are ignored because they do not describe the source's
/// sampling interval. Returns `None` when no positive gap exists. Latency
/// percentiles are `None` when no sample reports staleness.
pub fn sampling_stats(tracks: &[FlightTrack]) -> Option<SamplingStats> {
    let sample_count = tracks
        .iter()
        .map(|track| track.samples.len())
        .sum::<usize>();
    let mut intervals = Vec::new();
    let mut latencies = Vec::new();
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
        for sample in &track.samples {
            if let Some(latency) = sample.latency_s
                && latency.is_finite()
                && latency >= 0.0
            {
                latencies.push(latency);
            }
        }
    }
    let mut stats = stats_from_intervals(sample_count, &mut intervals)?;
    if let Some((median, p90)) = sorted_median_p90(&mut latencies) {
        stats.latency_median_s = Some(median);
        stats.latency_p90_s = Some(p90);
    }
    Some(stats)
}

/// Median and nearest-rank p90 of a set of measured values, sorted in place.
///
/// Returns `None` when the vector is empty. Shared by sampling gaps and
/// reported position staleness so both use identical rank rules.
pub(crate) fn sorted_median_p90(values: &mut [f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let median = if count % 2 == 1 {
        *values.get(count / 2)?
    } else {
        let lower = *values.get(count / 2 - 1)?;
        let upper = *values.get(count / 2)?;
        f64::midpoint(lower, upper)
    };
    let rank = ((count as f64) * 0.9).ceil() as usize;
    let p90_index = rank.saturating_sub(1).min(count - 1);
    let p90 = *values.get(p90_index)?;
    Some((median, p90))
}

/// Compute median and nearest-rank p90 from positive sampling intervals.
///
/// The interval vector is sorted in place. Returns `None` when it is empty.
pub(crate) fn stats_from_intervals(
    sample_count: usize,
    intervals: &mut [f64],
) -> Option<SamplingStats> {
    if intervals.is_empty() {
        return None;
    }
    let (median_s, p90_s) = sorted_median_p90(intervals)?;

    Some(SamplingStats {
        sample_count,
        interval_count: intervals.len(),
        median_s,
        p90_s,
        latency_median_s: None,
        latency_p90_s: None,
    })
}
