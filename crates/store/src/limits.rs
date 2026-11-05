//! Service limits (SDD section 11.3).

/// Initial service limits, provisional until v0 tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceLimits {
    /// Maximum upload size in bytes (25 MiB).
    pub max_upload_bytes: u64,
    /// Maximum public-demo clip length in seconds.
    pub max_service_clip_s: f64,
    /// Maximum local CLI clip length in seconds.
    pub max_research_clip_s: f64,
    /// Jobs allowed per IP address per 24 hours.
    pub jobs_per_ip_per_day: u32,
    /// Jobs allowed per anonymous session.
    pub jobs_per_session: u32,
    /// Media retention after job end in seconds (1 hour).
    pub media_retention_s: u64,
    /// Result retention after job end in seconds (24 hours).
    pub result_retention_s: u64,
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            max_upload_bytes: 26_214_400,
            max_service_clip_s: 90.0,
            max_research_clip_s: 600.0,
            jobs_per_ip_per_day: 10,
            jobs_per_session: 10,
            media_retention_s: 3_600,
            result_retention_s: 86_400,
        }
    }
}
