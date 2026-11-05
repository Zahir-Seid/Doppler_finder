//! Run mode, wall-time budgets and public-output precision.

/// How the pipeline is being run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Service job behind the API; the public demo cap applies.
    Service,
    /// Local research or CLI job; the NFR-02 budget applies.
    Research,
}

impl RunMode {
    /// Wall-time limit in seconds (SDD section 11.3).
    ///
    /// Service jobs get the provisional 120 s limit. Research jobs get the
    /// NFR-02 budget of 300 s for a typical 10-minute clip.
    pub fn wall_time_limit_s(self) -> f64 {
        match self {
            Self::Service => 120.0,
            Self::Research => 300.0,
        }
    }
}

/// Output and budget configuration for one run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PipelineConfig {
    /// Service or research mode.
    pub mode: RunMode,
    /// Minimum side of a public output cell in metres (D-1, FR-OUT-03).
    pub public_cell_m: f64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            mode: RunMode::Research,
            public_cell_m: 2_000.0,
        }
    }
}
