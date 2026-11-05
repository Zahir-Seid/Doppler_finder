//! Retention windows (SDD section 9.5).

/// How long each class of data is kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetentionPolicy {
    /// Uploaded media, seconds after job end.
    pub media_s: u64,
    /// Result rows and figures, seconds after job end.
    pub results_s: u64,
    /// Job rows without location data, seconds.
    pub job_rows_s: u64,
    /// Rate-limit rows, seconds.
    pub rate_limit_s: u64,
    /// Log retention, seconds.
    pub logs_s: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            media_s: 3_600,
            results_s: 86_400,
            job_rows_s: 2_592_000,
            rate_limit_s: 172_800,
            logs_s: 1_209_600,
        }
    }
}
