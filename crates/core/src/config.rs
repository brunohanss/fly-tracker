use crate::{Confidence, FrameSize};
use serde::{Deserialize, Serialize};
use thiserror::Error;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub safety_backend: SafetyBackend,
    #[serde(default)]
    pub processing: ProcessingConfig,
    pub version: u32,
    pub frame_size: FrameSize,
    pub max_frame_age_us: u64,
    pub max_safety_age_us: u64,
    pub minimum_clear_confidence: Confidence,
    pub max_tracks: usize,
    pub max_detections: usize,
    pub telemetry_capacity: usize,
    pub history_capacity: usize,
}
/// Image inference provides presence evidence. It does not establish clearance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case", deny_unknown_fields)]
pub enum SafetyBackend {
    #[default]
    Unavailable,
    Fixture,
    Nanodet {
        python: std::path::PathBuf,
        worker: std::path::PathBuf,
        param: std::path::PathBuf,
        weights: std::path::PathBuf,
        hazard_threshold: Confidence,
        timeout_us: u64,
        threads: u8,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingConfig {
    /// Simulated virtual aiming time. Replay source time controls the timer.
    #[serde(default = "default_virtual_aim_delay")]
    pub virtual_aim_delay_us: u64,
    /// Hold a selected virtual target for this source-time interval. Zero cycles each frame.
    #[serde(default)]
    pub virtual_aim_dwell_us: u64,
    #[serde(default = "default_stationary")]
    pub stationary_detection: bool,
    #[serde(default)]
    pub detection_region: Option<PixelRegion>,
    #[serde(default = "default_neighbors")]
    pub morphology_min_neighbors: u8,
    pub detection_threshold: u8,
    pub min_area: u32,
    pub max_area: u32,
    pub association_radius: f64,
    pub confirmation_hits: u32,
    pub remove_after_misses: u32,
    pub velocity_time_constant_s: f64,
    pub prediction_horizon_us: u64,
}
impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            virtual_aim_delay_us: default_virtual_aim_delay(),
            virtual_aim_dwell_us: 0,
            stationary_detection: true,
            detection_region: None,
            morphology_min_neighbors: 1,
            detection_threshold: 35,
            min_area: 3,
            max_area: 64,
            association_radius: 20.0,
            confirmation_hits: 3,
            remove_after_misses: 8,
            velocity_time_constant_s: 0.03,
            prediction_horizon_us: 10_000,
        }
    }
}
/// Insect detection region. Safety always receives the complete frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl PixelRegion {
    pub fn valid(self, size: FrameSize) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|x| x <= size.width())
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|y| y <= size.height())
    }
    pub fn contains(self, x: usize, y: usize) -> bool {
        (u64::from(self.x)..u64::from(self.x) + u64::from(self.width)).contains(&(x as u64))
            && (u64::from(self.y)..u64::from(self.y) + u64::from(self.height)).contains(&(y as u64))
    }
}
fn default_stationary() -> bool {
    true
}
fn default_virtual_aim_delay() -> u64 {
    500_000
}
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid JSON configuration: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid configuration: {0}")]
    Invalid(&'static str),
}
impl Default for Config {
    fn default() -> Self {
        Self {
            safety_backend: SafetyBackend::default(),
            processing: ProcessingConfig::default(),
            version: 1,
            frame_size: FrameSize::new(320, 240)
                .unwrap_or_else(|_| unreachable!("static valid size")),
            max_frame_age_us: 50_000,
            max_safety_age_us: 50_000,
            minimum_clear_confidence: Confidence::new(0.99)
                .unwrap_or_else(|_| unreachable!("static valid confidence")),
            max_tracks: 32,
            max_detections: 128,
            telemetry_capacity: 2,
            history_capacity: 1024,
        }
    }
}
impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 65536 {
            return Err(ConfigError::Invalid("configuration exceeds 64KiB"));
        }
        let config: Self = serde_json::from_reader(file)?;
        config.validate()?;
        Ok(config)
    }
    pub fn parse(json: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_json::from_str(json)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let SafetyBackend::Nanodet {
            python,
            worker,
            param,
            weights,
            hazard_threshold,
            timeout_us,
            threads,
        } = &self.safety_backend
            && ([python, worker, param, weights]
                .iter()
                .any(|path| path.as_os_str().is_empty())
                || hazard_threshold.get() <= 0.0
                || !(1..=60_000_000).contains(timeout_us)
                || !(1..=4).contains(threads))
        {
            return Err(ConfigError::Invalid("invalid NanoDet settings"));
        }
        let p = &self.processing;
        if p.detection_region
            .is_some_and(|region| !region.valid(self.frame_size))
        {
            return Err(ConfigError::Invalid("detection region outside frame"));
        }
        if p.detection_threshold == 0
            || p.virtual_aim_delay_us > 10_000_000
            || p.virtual_aim_dwell_us > 1_000_000
            || p.morphology_min_neighbors > 8
            || p.min_area == 0
            || p.max_area < p.min_area
            || !p.association_radius.is_finite()
            || p.association_radius <= 0.0
            || p.association_radius > 1_000_000.0
            || p.confirmation_hits == 0
            || p.remove_after_misses == 0
            || !p.velocity_time_constant_s.is_finite()
            || p.velocity_time_constant_s <= 0.0
            || !(1e-6..=10.0).contains(&p.velocity_time_constant_s)
            || p.prediction_horizon_us > 20_000
        {
            return Err(ConfigError::Invalid(
                "invalid processing parameters or horizon >20ms",
            ));
        }
        if self.version != 1 {
            return Err(ConfigError::Invalid("unsupported version"));
        }
        if self.max_frame_age_us == 0
            || self.max_safety_age_us == 0
            || self.max_frame_age_us > 1_000_000
            || self.max_safety_age_us > 1_000_000
        {
            return Err(ConfigError::Invalid(
                "freshness must be 1..=1000000 microseconds",
            ));
        }
        if self.minimum_clear_confidence.get() < 0.5 {
            return Err(ConfigError::Invalid("clear confidence must be >= 0.5"));
        }
        for capacity in [
            self.max_tracks,
            self.max_detections,
            self.telemetry_capacity,
            self.history_capacity,
        ] {
            if !(1..=65536).contains(&capacity) {
                return Err(ConfigError::Invalid("capacities must be 1..=65536"));
            }
        }
        Ok(())
    }
}
fn default_neighbors() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_schema_and_capacities() -> Result<(), ConfigError> {
        let mut config = Config::default();
        Config::parse(&serde_json::to_string(&config)?)?;
        config.max_tracks = 0;
        assert!(config.validate().is_err());
        config.max_tracks = 32;
        config.version = 2;
        assert!(config.validate().is_err());
        assert!(Config::parse("{}").is_err());
        Ok(())
    }
    #[test]
    fn virtual_delay_defaults_for_older_configs_and_rejects_excessive_values()
    -> Result<(), ConfigError> {
        let mut config = Config::default();
        assert_eq!(config.processing.virtual_aim_delay_us, 500_000);
        let mut value = serde_json::to_value(&config)?;
        value["processing"]
            .as_object_mut()
            .ok_or(ConfigError::Invalid("processing object"))?
            .remove("virtual_aim_delay_us");
        assert_eq!(
            Config::parse(&value.to_string())?
                .processing
                .virtual_aim_delay_us,
            500_000
        );
        config.processing.virtual_aim_delay_us = 10_000_001;
        assert!(config.validate().is_err());
        config.processing.virtual_aim_delay_us = 0;
        config.validate()?;
        Ok(())
    }
}
