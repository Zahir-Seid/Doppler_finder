//! Source/region support flags (SDD T-4, FR-DA-01).
//!
//! An empty track set is not proof of coverage: it can also mean the archive
//! has no receivers over the region or date. [`assess_coverage`] reduces an
//! ingest result to a [`CoverageReport`] so downstream stages never treat a
//! silent region as a complete one. The SDD 10.3 error codes
//! `region_unsupported` and `no_adsb_coverage` map from
//! [`RegionSupport::Insufficient`] and [`RegionSupport::Unsupported`]
//! respectively; the mapping itself belongs to `api`.
//!
//! Until T-4 is measured, regions without evidence stay unsupported: only a
//! non-empty ingest with measured sampling inside the provisional bound earns
//! [`RegionSupport::Supported`].

use crate::stats::stats_from_intervals;
use crate::tracks::FlightTrack;

/// Largest median sampling gap still counted as usable coverage, in seconds.
///
/// PROVISIONAL(T-4): fixed pending the measured per-source sampling
/// statistics. Ten seconds is generous against the ~1 s cadence of the open
/// archives; it exists to catch a region served only by sparse receivers,
/// not to grade normal jitter.
pub const MAX_SUPPORTED_MEDIAN_GAP_S: f64 = 10.0;

/// Whether a region's measured ADS-B coverage can support a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSupport {
    /// Non-empty ingest whose median sampling gap meets the provisional bound.
    Supported,
    /// Tracks exist but measured sampling is too sparse to trust; maps to the
    /// `region_unsupported` result code downstream.
    Insufficient,
    /// The query returned no usable tracks, so coverage is unproven; maps to
    /// `no_adsb_coverage` downstream. This is the T-4 default for unmeasured
    /// regions such as Addis Ababa.
    Unsupported,
}

/// Coverage assessment for one ingest result.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageReport {
    /// The support verdict for the queried region and window.
    pub support: RegionSupport,
    /// Number of tracks the ingest returned.
    pub track_count: usize,
    /// Number of track samples the ingest returned.
    pub sample_count: usize,
    /// Median positive sampling gap in seconds across all tracks, when any
    /// gap was measured.
    pub median_gap_s: Option<f64>,
    /// Largest positive sampling gap in seconds, when any gap was measured.
    pub max_gap_s: Option<f64>,
}

/// Assess coverage from the tracks an ingest produced for a query.
///
/// The assessment uses only what was measured: an empty result is
/// [`RegionSupport::Unsupported`] rather than a complete empty set, and a
/// result whose median gap exceeds [`MAX_SUPPORTED_MEDIAN_GAP_S`] is
/// [`RegionSupport::Insufficient`]. Tracks without any positive gap (single
/// samples) cannot prove sampling and stay [`RegionSupport::Insufficient`]
/// unless they are empty, which is [`RegionSupport::Unsupported`].
pub fn assess_coverage(tracks: &[FlightTrack]) -> CoverageReport {
    let sample_count = tracks
        .iter()
        .map(|track| track.samples.len())
        .sum::<usize>();
    let mut gaps = Vec::new();
    for track in tracks {
        for pair in track.samples.windows(2) {
            let (Some(first), Some(second)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let gap_s = (second.utc.0 - first.utc.0) as f64 / 1_000.0;
            if gap_s.is_finite() && gap_s > 0.0 {
                gaps.push(gap_s);
            }
        }
    }
    let stats = stats_from_intervals(sample_count, &mut gaps);
    let median_gap_s = stats.as_ref().map(|stats| stats.median_s);
    let max_gap_s = gaps.iter().copied().reduce(f64::max);

    let support = if tracks.is_empty() {
        RegionSupport::Unsupported
    } else if median_gap_s.is_some_and(|median| median <= MAX_SUPPORTED_MEDIAN_GAP_S) {
        RegionSupport::Supported
    } else {
        RegionSupport::Insufficient
    };

    CoverageReport {
        support,
        track_count: tracks.len(),
        sample_count,
        median_gap_s,
        max_gap_s,
    }
}
