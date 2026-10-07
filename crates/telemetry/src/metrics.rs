use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Invalid metric or report: {0}")]
    Invalid(&'static str),
    #[error("Report JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Report I/O failed: {0}")]
    Io(#[from] std::io::Error),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Distribution {
    pub count: usize,
    pub total_samples: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub maximum: f64,
    pub mean: f64,
    pub rms: f64,
}
/// Exact percentiles over a bounded last-N sample window, with lifetime counts.
/// Aggregation occurs when requested, never inside draw code.
pub struct Series {
    samples: VecDeque<f64>,
    scratch: Vec<f64>,
    capacity: usize,
    total: u64,
}
impl Series {
    pub fn new(capacity: usize) -> Result<Self, MetricsError> {
        if !(1..=65536).contains(&capacity) {
            return Err(MetricsError::Invalid("capacity must be 1..=65536"));
        }
        Ok(Self {
            samples: VecDeque::with_capacity(capacity),
            scratch: Vec::with_capacity(capacity),
            capacity,
            total: 0,
        })
    }
    pub fn push(&mut self, value: f64) -> Result<(), MetricsError> {
        if !value.is_finite() || !(0.0..=1e12).contains(&value) {
            return Err(MetricsError::Invalid(
                "sample must be finite and in [0, 1e12]",
            ));
        }
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
        self.total = self.total.saturating_add(1);
        Ok(())
    }
    pub fn distribution(&mut self) -> Option<Distribution> {
        if self.samples.is_empty() {
            return None;
        }
        self.scratch.clear();
        self.scratch.extend(self.samples.iter().copied());
        self.scratch.sort_by(f64::total_cmp);
        let count = self.scratch.len();
        let percentile =
            |percent: usize| self.scratch[(count * percent).div_ceil(100).saturating_sub(1)];
        Some(Distribution {
            count,
            total_samples: self.total,
            p50: percentile(50),
            p95: percentile(95),
            p99: percentile(99),
            maximum: self.scratch[count - 1],
            mean: self.scratch.iter().sum::<f64>() / count as f64,
            rms: (self.scratch.iter().map(|v| v * v).sum::<f64>() / count as f64).sqrt(),
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Capture,
    Safety,
    Detection,
    Tracking,
    Prediction,
    Aiming,
    FrameToCommand,
    FrameProcessing,
}
impl Stage {
    pub const ALL: [Self; 8] = [
        Self::Capture,
        Self::Safety,
        Self::Detection,
        Self::Tracking,
        Self::Prediction,
        Self::Aiming,
        Self::FrameToCommand,
        Self::FrameProcessing,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
}
pub struct Latencies {
    series: Vec<Series>,
    windows: crate::windows::TimeWindows,
    timestamp: fly_core::FrameTimestamp,
}
impl Latencies {
    pub fn new(capacity: usize) -> Result<Self, MetricsError> {
        Ok(Self {
            windows: crate::windows::TimeWindows::default(),
            timestamp: fly_core::FrameTimestamp(0),
            series: (0..8)
                .map(|_| Series::new(capacity))
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn record(
        &mut self,
        stage: Stage,
        duration: std::time::Duration,
    ) -> Result<(), MetricsError> {
        let micros = duration.as_secs_f64() * 1_000_000.0;
        self.series[stage.index()].push(micros)?;
        self.windows.record(self.timestamp, stage, micros)
    }
    pub fn distributions(&mut self) -> Vec<Option<Distribution>> {
        self.series.iter_mut().map(Series::distribution).collect()
    }
    pub fn set_timestamp(&mut self, timestamp: fly_core::FrameTimestamp) {
        self.timestamp = timestamp;
    }
    pub fn time_windows(&self) -> Vec<crate::windows::WindowDistribution> {
        self.windows.distributions()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub calibration_rejections: u64,
    pub processed_frames: u64,
    pub dropped_frames: u64,
    pub detections: u64,
    pub acquired_targets: u64,
    pub lost_targets: u64,
    pub reacquisitions: u64,
    pub lockouts: u64,
    pub lockout_frames: u64,
    pub issued_aims: u64,
    pub suppressed_aims: u64,
    pub truth_frames: u64,
    pub false_positives: Option<u64>,
    pub false_negatives: Option<u64>,
    pub truth_target_observations: u64,
    pub tracked_truth_observations: u64,
    pub id_switches: u64,
    pub safety_positive_frames: u64,
    pub unsafe_commands: u64,
    pub prediction_samples_dropped: u64,
    pub detection_capacity_frames: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    pub dataset: String,
    pub config_fingerprint: String,
    pub hardware: String,
    pub build: String,
    pub metric_version: u32,
    pub git_commit: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMetrics {
    #[serde(default)]
    pub time_windows: Vec<crate::windows::WindowDistribution>,
    pub version: u32,
    pub completed_unix_seconds: u64,
    pub identity: RunIdentity,
    pub sample_capacity: usize,
    pub window: String,
    pub latency_unit: String,
    pub counters: Counters,
    pub latency: Vec<Option<Distribution>>,
    pub detection_error_pixels: Option<Distribution>,
    /// Current / +5ms / +10ms / +20ms; interpolated source truth can include occluded positions.
    pub prediction_error_pixels: Vec<Option<Distribution>>,
    pub virtual_aim_error_pixels: Option<Distribution>,
}
impl RunMetrics {
    pub fn validate(&self) -> Result<(), MetricsError> {
        if self.identity.dataset.len() > 4096
            || self.identity.config_fingerprint.len() != 16
            || self.window.len() > 1024
            || self.time_windows.len() > 5
        {
            return Err(MetricsError::Invalid("unbounded report metadata"));
        }
        if (self.counters.truth_frames == 0
            && (self.counters.false_positives.is_some() || self.counters.false_negatives.is_some()))
            || (self.counters.truth_frames > 0
                && (self.counters.false_positives.is_none()
                    || self.counters.false_negatives.is_none()))
        {
            return Err(MetricsError::Invalid(
                "ground-truth metrics must be unavailable without truth",
            ));
        }
        for (index, window) in self.time_windows.iter().enumerate() {
            if window.seconds != [Some(60), Some(300), Some(900), Some(3600), None][index]
                || window.latency.len() != 8
                || window.quantiles.len() > 1024
            {
                return Err(MetricsError::Invalid("invalid time windows"));
            }
            for d in window.latency.iter().flatten() {
                if d.count == 0
                    || d.total_samples < d.count as u64
                    || [d.p50, d.p95, d.p99, d.maximum, d.mean, d.rms]
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1e12).contains(v))
                    || d.p50 > d.p95
                    || d.p95 > d.p99
                    || d.p99 > d.maximum
                {
                    return Err(MetricsError::Invalid("invalid window distribution"));
                }
            }
        }
        if self.version != 1
            || self.identity.metric_version != 1
            || self.identity.dataset.is_empty()
            || !(1..=65536).contains(&self.sample_capacity)
            || self.latency.len() != 8
            || self.prediction_error_pixels.len() != 4
            || self.latency_unit != "microseconds"
        {
            return Err(MetricsError::Invalid("unsupported schema"));
        }
        for distribution in self
            .latency
            .iter()
            .chain(&self.prediction_error_pixels)
            .chain([&self.detection_error_pixels, &self.virtual_aim_error_pixels])
            .flatten()
        {
            if distribution.count == 0
                || distribution.count > self.sample_capacity
                || distribution.total_samples < distribution.count as u64
                || [
                    distribution.p50,
                    distribution.p95,
                    distribution.p99,
                    distribution.maximum,
                    distribution.mean,
                    distribution.rms,
                ]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0 || *v > 1e12)
                || distribution.p50 > distribution.p95
                || distribution.p95 > distribution.p99
                || distribution.p99 > distribution.maximum
            {
                return Err(MetricsError::Invalid("invalid distribution"));
            }
        }
        Ok(())
    }
    pub fn comparable(&self, other: &Self) -> bool {
        self.identity.dataset == other.identity.dataset
            && !self.identity.hardware.starts_with("unverified:")
            && self.identity.config_fingerprint == other.identity.config_fingerprint
            && self.identity.hardware == other.identity.hardware
            && self.identity.build == other.identity.build
            && self.identity.metric_version == other.identity.metric_version
            && self.sample_capacity == other.sample_capacity
            && self.window == other.window
    }
    pub fn load(path: &Path) -> Result<Self, MetricsError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 4 * 1024 * 1024 {
            return Err(MetricsError::Invalid("report too large"));
        }
        let report: Self = serde_json::from_reader(file)?;
        report.validate()?;
        Ok(report)
    }
    pub fn save(&self, path: &Path) -> Result<(), MetricsError> {
        self.validate()?;
        if path.exists() {
            return Err(MetricsError::Invalid(
                "report already exists; select a new path",
            ));
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            // Linking publishes atomically and fails if another writer created the target.
            std::fs::hard_link(&temp, path)?;
            std::fs::remove_file(&temp)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result?;
        Ok(())
    }
}
/// Stable non-cryptographic identity for versioned configuration and algorithm parameters.
pub fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(14695981039346656037_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    });
    format!("{hash:016x}")
}
pub fn percentage_change(current: f64, previous: f64) -> Option<f64> {
    if current.is_finite() && previous.is_finite() && previous != 0.0 {
        Some((current - previous) / previous * 100.0)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_exact_percentiles_reject_invalid_values() -> Result<(), MetricsError> {
        let mut series = Series::new(3)?;
        assert!(series.distribution().is_none());
        for value in [1.0, 2.0, 3.0, 4.0] {
            series.push(value)?;
        }
        let stats = series
            .distribution()
            .ok_or(MetricsError::Invalid("missing samples"))?;
        assert_eq!(stats.count, 3);
        assert_eq!(stats.total_samples, 4);
        assert_eq!(stats.p50, 3.0);
        assert_eq!(stats.p99, 4.0);
        assert!(series.push(f64::NAN).is_err());
        assert!(series.push(-1.0).is_err());
        assert_eq!(percentage_change(1.0, 0.0), None);
        assert_eq!(percentage_change(2.0, 1.0), Some(100.0));
        Ok(())
    }
}
